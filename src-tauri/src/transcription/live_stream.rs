//! Live transcription against a remote streaming recogniser.
//!
//! Deliberately a separate loop from `live.rs` rather than a branch inside it.
//! The two want different cadences — whisper is given three seconds of audio at
//! a time because a short window transcribes badly, whereas a streaming
//! recogniser wants ~100 ms frames paced like real time — and the local path is
//! the one that must never break. They share the capture layer, which is where
//! the hard-won work actually lives: the same `take_audio_buffer()` and
//! `take_system_audio_samples()` drains, the same channel folding and
//! resampling, the same gain treatment.
//!
//! Only one of the two may run at a time: both drain the same buffers, so
//! running both would have them stealing each other's audio. `is_running` on
//! the shared `LiveTranscriptionState` enforces that, and means the existing
//! `stop_live_transcription` command stops this path too.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager};
use tokio::time::{interval, Duration};

use crate::audio::{take_system_audio_samples, RecordingPhase, RecordingState};
use crate::db::{models::NewTranscriptSegment, Database};
use crate::transcription::live::{
    resample, AudioSource, LiveTranscriptionState, TranscriptionUpdateEvent, MIC_MAX_GAIN,
    MIC_TARGET_PEAK,
};
use crate::transcription::streaming::{
    self, Recognised, StreamingSession, CHUNK_SAMPLES, SAMPLE_RATE,
};
use crate::transcription::transcriber::TranscriptionSegment;
use crate::transcription::TranscriptionError;

/// How often the capture buffers are drained and pushed to the sockets.
///
/// One frame's worth. The recogniser is a streaming model and the reference
/// client paces sends in real time; firing a backlog at it as fast as the
/// socket accepts is explicitly not the tested path.
const TICK: Duration = Duration::from_millis(streaming::CHUNK_MS as u64);

/// Keeps a quiet microphone usable without flattening the signal.
///
/// `live.rs` normalises each three-second window in place, which is fine at
/// that length. At 100 ms it would be actively harmful: every frame would be
/// scaled to the same peak independently, so silence would be amplified to
/// match speech and the recogniser would lose the dynamics it uses to find word
/// boundaries. Instead the gain is derived from recent audio and applied to the
/// current frame — no added latency, and the level stays continuous across
/// frames.
///
/// The cap is what preserves the silence/speech distinction, exactly as it does
/// in the batch path: near-silence cannot be lifted far enough to look like
/// speech.
#[derive(Debug, Clone)]
pub struct MicGain {
    recent_peak: f32,
    target: f32,
    max_gain: f32,
}

impl Default for MicGain {
    fn default() -> Self {
        Self::new(MIC_TARGET_PEAK, MIC_MAX_GAIN)
    }
}

impl MicGain {
    pub fn new(target: f32, max_gain: f32) -> Self {
        Self {
            recent_peak: 0.0,
            target,
            max_gain,
        }
    }

    /// Fold a frame's peak into the running estimate and return the gain to
    /// apply to it.
    ///
    /// Rises immediately on a louder frame and decays slowly on quieter ones, so
    /// a pause between sentences does not wind the gain up into the noise floor
    /// and then clip the next word.
    pub fn observe(&mut self, samples: &[f32]) -> f32 {
        let peak = samples.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        self.recent_peak = if peak > self.recent_peak {
            peak
        } else {
            // ~1 s time constant at 100 ms frames.
            self.recent_peak * 0.9 + peak * 0.1
        };
        if self.recent_peak <= f32::EPSILON {
            return 1.0;
        }
        (self.target / self.recent_peak).clamp(1.0, self.max_gain)
    }

    /// Apply the gain to a frame in place.
    pub fn apply(&mut self, samples: &mut [f32]) {
        let gain = self.observe(samples);
        if (gain - 1.0).abs() > f32::EPSILON {
            for s in samples.iter_mut() {
                *s = (*s * gain).clamp(-1.0, 1.0);
            }
        }
    }
}

/// Carries leftover samples between ticks so the wire always sees whole frames.
///
/// A drain rarely lands on a frame boundary — the capture callback fills at the
/// device's own cadence, not ours. Sending the remainder as a short frame would
/// make every tick a partial one and the sample count on the wire would stop
/// matching the clock the timestamps come from.
#[derive(Debug, Default, Clone)]
pub struct FrameBuffer {
    pending: Vec<f32>,
}

impl FrameBuffer {
    /// Add samples and take out every whole frame now available.
    pub fn push(&mut self, samples: &[f32]) -> Vec<Vec<f32>> {
        self.pending.extend_from_slice(samples);
        let mut frames = Vec::new();
        while self.pending.len() >= CHUNK_SAMPLES {
            frames.push(self.pending.drain(..CHUNK_SAMPLES).collect());
        }
        frames
    }

    /// Take what is left, padded to a whole frame.
    ///
    /// Called once when recording stops: the tail is real audio someone spoke,
    /// and dropping it would truncate the last word of the meeting.
    pub fn flush(&mut self) -> Option<Vec<f32>> {
        if self.pending.is_empty() {
            return None;
        }
        let mut last: Vec<f32> = std::mem::take(&mut self.pending);
        last.resize(CHUNK_SAMPLES, 0.0);
        Some(last)
    }
}

/// Fold interleaved capture into the mono 16 kHz the recogniser expects.
pub fn to_mono_16k(samples: Vec<f32>, rate: u32, channels: usize) -> Vec<f32> {
    let mono: Vec<f32> = if channels > 1 {
        samples
            .chunks(channels)
            .map(|c| c.iter().sum::<f32>() / channels as f32)
            .collect()
    } else {
        samples
    };
    if rate != SAMPLE_RATE as u32 && rate > 0 {
        resample(&mono, rate, SAMPLE_RATE as u32)
    } else {
        mono
    }
}

/// The track label a segment gets.
///
/// Not `None`, even though this recogniser does not diarize. Which socket a
/// segment came back on *is* attribution — it is the same two-way distinction
/// the local path records, and it is the reason this backend opens two sockets
/// instead of mixing the tracks. Calling it unknown would throw away something
/// the app actually knows.
pub fn speaker_for(source: AudioSource) -> &'static str {
    match source {
        AudioSource::Mic => "You",
        AudioSource::System => "Others",
    }
}

/// Start live transcription against the streaming recogniser.
///
/// Both sockets are opened before anything is marked running, so a recogniser
/// that is unreachable fails here — visibly, before a recording is under way —
/// rather than part-way through a meeting.
pub async fn start_streaming_transcription(
    app: AppHandle,
    note_id: String,
    ws_url: String,
    recording_state: Arc<RecordingState>,
    live_state: Arc<LiveTranscriptionState>,
) -> Result<(), TranscriptionError> {
    if live_state.is_running.swap(true, Ordering::SeqCst) {
        return Err(TranscriptionError::AlreadyTranscribing);
    }

    // Any early return past this point has to put the flag back, or live
    // transcription can never be started again without restarting the app.
    let (mic_session, mic_rx) = match streaming::connect(&ws_url, "microphone").await {
        Ok(pair) => pair,
        Err(e) => {
            live_state.is_running.store(false, Ordering::SeqCst);
            return Err(TranscriptionError::TranscriptionFailed(e));
        }
    };
    let (system_session, system_rx) = match streaming::connect(&ws_url, "system audio").await {
        Ok(pair) => pair,
        Err(e) => {
            live_state.is_running.store(false, Ordering::SeqCst);
            return Err(TranscriptionError::TranscriptionFailed(e));
        }
    };

    *live_state.mic_time_offset.lock().await = 0.0;
    *live_state.system_time_offset.lock().await = 0.0;
    live_state.segments.lock().await.clear();

    spawn_reader(
        app.clone(),
        note_id.clone(),
        live_state.clone(),
        mic_rx,
        AudioSource::Mic,
    );
    spawn_reader(
        app.clone(),
        note_id.clone(),
        live_state.clone(),
        system_rx,
        AudioSource::System,
    );

    tokio::spawn(feed_loop(
        note_id,
        recording_state,
        live_state,
        mic_session,
        system_session,
    ));

    Ok(())
}

/// Drains capture into the two sockets, one frame at a time.
async fn feed_loop(
    note_id: String,
    recording_state: Arc<RecordingState>,
    live_state: Arc<LiveTranscriptionState>,
    mic_session: StreamingSession,
    system_session: StreamingSession,
) {
    let mut ticker = interval(TICK);
    let mut mic_frames = FrameBuffer::default();
    let mut system_frames = FrameBuffer::default();
    let mut gain = MicGain::default();

    loop {
        ticker.tick().await;

        if !live_state.is_running.load(Ordering::SeqCst) {
            break;
        }
        if recording_state.get_phase() != RecordingPhase::Recording {
            break;
        }

        // A socket that has gone stops the whole session rather than leaving
        // the other one running. Half a meeting transcribed, with no sign that
        // the other half was lost, is worse than a clean stop: the transcript
        // would look complete.
        if !mic_session.is_alive() || !system_session.is_alive() {
            println!("[stream] a recogniser socket closed; stopping live transcription for {note_id}");
            live_state.is_running.store(false, Ordering::SeqCst);
            break;
        }

        let mic_samples = recording_state.take_audio_buffer();
        if !mic_samples.is_empty() {
            let rate = recording_state.sample_rate.load(Ordering::SeqCst);
            let ch = recording_state.channels.load(Ordering::SeqCst) as usize;
            if rate > 0 && ch > 0 {
                let mono = to_mono_16k(mic_samples, rate, ch);
                for mut frame in mic_frames.push(&mono) {
                    gain.apply(&mut frame);
                    if !mic_session.send(streaming::to_s16le(&frame)) {
                        println!("[stream] the microphone recogniser stopped accepting audio");
                        break;
                    }
                }
            }
        }

        // System audio arrives already mono at 16 kHz, and gets no gain: it is
        // whatever the far end sent, at the level they sent it.
        let system_samples = take_system_audio_samples();
        if !system_samples.is_empty() {
            for frame in system_frames.push(&system_samples) {
                if !system_session.send(streaming::to_s16le(&frame)) {
                    println!("[stream] the system-audio recogniser stopped accepting audio");
                    break;
                }
            }
        }
    }

    // Push the tails, then drop the sessions — which closes the sockets, and
    // the writer sends the finalize frame on its way out so the last utterance
    // still comes back.
    if let Some(mut frame) = mic_frames.flush() {
        gain.apply(&mut frame);
        mic_session.send(streaming::to_s16le(&frame));
    }
    if let Some(frame) = system_frames.flush() {
        system_session.send(streaming::to_s16le(&frame));
    }

    // Give the finals a moment to come back before the readers see the socket
    // close. Without it, speech already spoken would be dropped on the floor.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    drop(mic_session);
    drop(system_session);
    live_state.is_running.store(false, Ordering::SeqCst);
}

/// Consumes one track's recognitions: partials to the UI, finals to the DB.
fn spawn_reader(
    app: AppHandle,
    note_id: String,
    live_state: Arc<LiveTranscriptionState>,
    mut rx: tokio::sync::mpsc::Receiver<Recognised>,
    source: AudioSource,
) {
    tokio::spawn(async move {
        while let Some(item) = rx.recv().await {
            match item {
                Recognised::Partial { text } => {
                    if text.trim().is_empty() {
                        continue;
                    }
                    // Partials are redrawn in place and never stored: they are
                    // the recogniser's current guess, and half of them are
                    // wrong by design.
                    let segment = TranscriptionSegment {
                        start_time: 0.0,
                        end_time: 0.0,
                        text,
                        speaker: Some(speaker_for(source).to_string()),
                    };
                    let _ = app.emit(
                        "transcription-update",
                        TranscriptionUpdateEvent {
                            note_id: note_id.clone(),
                            segments: vec![segment],
                            is_final: false,
                            audio_source: source,
                        },
                    );
                }
                Recognised::Final {
                    text,
                    start_time,
                    end_time,
                } => {
                    let segment = TranscriptionSegment {
                        start_time,
                        end_time,
                        text: text.clone(),
                        speaker: Some(speaker_for(source).to_string()),
                    };

                    // Stored before it is announced: the event is what the UI
                    // draws, and drawing a segment that failed to persist would
                    // show the user a transcript they do not have.
                    let db = app.state::<Database>();
                    let row = NewTranscriptSegment::new(
                        note_id.clone(),
                        start_time,
                        end_time,
                        text.clone(),
                    )
                    .with_speaker(Some(speaker_for(source).to_string()))
                    .with_source_type("live");
                    if let Err(e) = db.add_transcript_segment(&row) {
                        eprintln!("[stream] could not save a segment: {e}");
                        continue;
                    }

                    live_state.segments.lock().await.push(segment.clone());

                    let _ = app.emit(
                        "transcription-update",
                        TranscriptionUpdateEvent {
                            note_id: note_id.clone(),
                            segments: vec![segment],
                            is_final: true,
                            audio_source: source,
                        },
                    );
                }
                Recognised::Disconnected { reason } => {
                    // The feed loop notices this too, through `is_alive`. Doing
                    // it here as well means a socket that dies while no audio
                    // is being captured still stops the session.
                    println!("[stream] {reason}");
                    live_state.is_running.store(false, Ordering::SeqCst);
                    let _ = app.emit("transcription-stream-lost", reason);
                    break;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_whole_and_the_remainder_carries_over() {
        let mut buf = FrameBuffer::default();
        // A drain that lands mid-frame.
        let frames = buf.push(&vec![0.5; CHUNK_SAMPLES + 100]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].len(), CHUNK_SAMPLES);

        // The leftover 100 is not lost — it heads the next frame.
        let frames = buf.push(&vec![0.25; CHUNK_SAMPLES - 100]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0][0], 0.5, "the carried-over samples come first");
        assert_eq!(frames[0][CHUNK_SAMPLES - 1], 0.25);
    }

    #[test]
    fn a_short_drain_yields_nothing_until_a_frame_is_whole() {
        let mut buf = FrameBuffer::default();
        assert!(buf.push(&vec![0.1; 10]).is_empty());
        assert!(buf.push(&vec![0.1; 10]).is_empty());
        assert_eq!(buf.pending.len(), 20);
    }

    #[test]
    fn the_tail_is_padded_rather_than_dropped() {
        // The last words of a meeting live here.
        let mut buf = FrameBuffer::default();
        buf.push(&vec![0.5; 400]);
        let tail = buf.flush().expect("a partial frame is still audio");
        assert_eq!(tail.len(), CHUNK_SAMPLES);
        assert_eq!(tail[399], 0.5);
        assert_eq!(tail[400], 0.0, "padded with silence, not with noise");
        assert!(buf.flush().is_none(), "flushing twice sends nothing twice");
    }

    #[test]
    fn a_quiet_mic_is_lifted_but_silence_is_not() {
        let mut gain = MicGain::default();
        let mut quiet = vec![0.03_f32; CHUNK_SAMPLES];
        gain.apply(&mut quiet);
        assert!(
            quiet[0] > 0.2,
            "a quiet mic should reach a usable level, got {}",
            quiet[0]
        );

        // Near-silence must not be amplified into something that looks like
        // speech — the cap is the only thing preventing that.
        let mut silence = MicGain::default();
        let mut hiss = vec![0.0005_f32; CHUNK_SAMPLES];
        silence.apply(&mut hiss);
        assert!(hiss[0] <= 0.0005 * MIC_MAX_GAIN + f32::EPSILON);
        assert!(hiss[0] < 0.02, "still below the speech threshold");
    }

    #[test]
    fn gain_does_not_wind_up_during_a_pause() {
        // The failure this guards: gain climbs through a silent gap between
        // sentences, then the next word arrives at full level and clips.
        let mut gain = MicGain::default();
        gain.apply(&mut vec![0.4_f32; CHUNK_SAMPLES]);
        for _ in 0..5 {
            gain.apply(&mut vec![0.0_f32; CHUNK_SAMPLES]);
        }
        let mut word = vec![0.4_f32; CHUNK_SAMPLES];
        gain.apply(&mut word);
        assert!(
            word[0] <= 1.0,
            "the signal must not be driven into clipping"
        );
        assert!(
            word[0] < 0.9,
            "gain wound up during the pause: {} after silence",
            word[0]
        );
    }

    #[test]
    fn loud_audio_is_left_alone() {
        let mut gain = MicGain::default();
        let mut loud = vec![0.9_f32; CHUNK_SAMPLES];
        gain.apply(&mut loud);
        assert_eq!(loud[0], 0.9, "gain never attenuates, only lifts");
    }

    #[test]
    fn stereo_capture_is_folded_and_resampled() {
        // 48 kHz stereo in, 16 kHz mono out — a third of the frames.
        let interleaved: Vec<f32> = (0..960).map(|i| if i % 2 == 0 { 1.0 } else { 0.0 }).collect();
        let out = to_mono_16k(interleaved, 48_000, 2);
        assert_eq!(out.len(), 160);
        assert!(out.iter().all(|s| (*s - 0.5).abs() < 0.01), "channels averaged");
    }

    #[test]
    fn mono_at_the_target_rate_passes_through_untouched() {
        let samples = vec![0.25_f32; CHUNK_SAMPLES];
        let out = to_mono_16k(samples.clone(), 16_000, 1);
        assert_eq!(out, samples);
    }

    #[test]
    fn the_track_a_segment_arrived_on_is_the_speaker() {
        // This recogniser does not diarize, but which socket answered is still
        // attribution, and it is why there are two.
        assert_eq!(speaker_for(AudioSource::Mic), "You");
        assert_eq!(speaker_for(AudioSource::System), "Others");
    }
}
