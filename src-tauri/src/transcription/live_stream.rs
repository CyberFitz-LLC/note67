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
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tauri::{AppHandle, Emitter, Manager};
use tokio::time::{interval, Duration};

use crate::audio::{take_system_audio_samples, RecordingPhase, RecordingState};
use crate::db::{models::NewTranscriptSegment, Database};
use crate::transcription::live::{
    resample, AudioSource, LiveTranscriptionState, TranscriptionUpdateEvent, MIC_MAX_GAIN,
    MIC_TARGET_PEAK,
};
use crate::transcription::streaming::{
    self, Recognised, SendOutcome, StreamingSession, CHUNK_SAMPLES, SAMPLE_RATE,
};
use crate::transcription::transcriber::TranscriptionSegment;
use crate::transcription::{is_echo_of_system, TranscriptionError};

/// How long a track may be quiet before its utterance is closed.
///
/// The natural boundary in speech, and the one that gives a transcript its
/// shape: without it a meeting is a single utterance, because this recogniser
/// only returns a final when asked.
const SILENCE_CLOSES_UTTERANCE: Duration = Duration::from_millis(900);

/// The longest an utterance may run before being closed regardless.
///
/// A backstop for someone who simply does not pause. Twenty seconds keeps
/// segments readable and keeps the recogniser's working state bounded — an
/// utterance that grows for an hour is what turns a crisp transcript into one
/// running minutes behind.
const MAX_UTTERANCE: Duration = Duration::from_secs(20);

/// Below this, a frame counts as silence for the purpose of closing an
/// utterance. Deliberately lower than the transcription voice gate: this only
/// decides where to put a boundary, so it should err towards hearing speech.
const SILENCE_RMS: f32 = 0.01;

/// Tracks when one socket's current utterance should be closed.
#[derive(Debug)]
struct Utterance {
    /// When speech was last heard on this track.
    last_voice: Option<Instant>,
    /// When the current utterance began.
    started: Option<Instant>,
}

impl Utterance {
    fn new() -> Self {
        Self {
            last_voice: None,
            started: None,
        }
    }

    /// Note a frame, and say whether the utterance should now be closed.
    fn observe(&mut self, rms: f32, now: Instant) -> bool {
        if rms > SILENCE_RMS {
            self.last_voice = Some(now);
            if self.started.is_none() {
                self.started = Some(now);
            }
            // Long enough that it must be broken somewhere, and here is as good
            // as anywhere.
            if self.started.is_some_and(|s| now.duration_since(s) >= MAX_UTTERANCE) {
                self.reset();
                return true;
            }
            return false;
        }

        // Silence. Only closes something that was actually open — otherwise a
        // quiet meeting would send a finalize every tick and ask the recogniser
        // for an endless run of empty finals.
        let open = self.started.is_some();
        let quiet_long_enough = self
            .last_voice
            .is_some_and(|v| now.duration_since(v) >= SILENCE_CLOSES_UTTERANCE);

        if open && quiet_long_enough {
            self.reset();
            return true;
        }
        false
    }

    fn reset(&mut self) {
        self.started = None;
        self.last_voice = None;
    }
}

/// What to subtract from a track's arrival time before placing an utterance.
///
/// The microphone path waits `ECHO_GRACE` before judging whether an utterance
/// was the room's speakers, so its finals arrive that much later than the
/// system track's for no reason to do with when they were said. Removing it
/// keeps the two tracks aligned with each other, which is what ordering
/// depends on.
fn latency_allowance(source: AudioSource) -> f64 {
    match source {
        AudioSource::Mic => ECHO_GRACE.as_secs_f64(),
        AudioSource::System => 0.0,
    }
}

/// Root-mean-square of a frame.
fn frame_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// How long a settled microphone utterance is held before it is judged an echo.
///
/// Both tracks go to the same recogniser, so the far end's words and the mic's
/// re-capture of them are recognised at similar speed — but not in a guaranteed
/// order. Without a pause, a mic echo can be judged against a window that does
/// not yet contain the system utterance it is echoing, and it survives.
///
/// Costs nothing visible: the partial is already on screen, and this only
/// delays the point at which it is written down.
const ECHO_GRACE: Duration = Duration::from_millis(600);

/// How much recent system audio is kept for echo comparison.
const ECHO_MEMORY_SECS: f64 = 30.0;

/// How often the capture buffers are drained and pushed to the sockets.
///
/// One frame's worth. The recogniser is a streaming model and the reference
/// client paces sends in real time; firing a backlog at it as fast as the
/// socket accepts is explicitly not the tested path.
const TICK: Duration = Duration::from_millis(streaming::CHUNK_MS as u64);

/// Recent system-audio utterances, for judging whether the microphone is
/// hearing the room's speakers rather than a person.
///
/// **Spans are measured by arrival, not by the track clocks.** Each clock counts
/// what that socket has been sent, and the two tracks are not fed at the same
/// rate — Windows loopback delivers nothing at all while no application is
/// playing audio, so the system track's absolute offset falls behind the
/// microphone's by however much silence has passed. Comparing those numbers
/// would put the two tracks in different time frames and the overlap test would
/// never fire.
///
/// What each clock *does* measure reliably is duration. So an utterance is
/// placed by when it came back, extending backwards by how long it ran, which
/// puts both tracks on one wall clock.
#[derive(Clone, Default)]
pub struct EchoWindow {
    /// (start, end, text), seconds since the session began.
    recent: Arc<Mutex<Vec<(f64, f64, String)>>>,
}

impl EchoWindow {
    /// Note a system utterance that ended `at` seconds into the session.
    pub fn record(&self, at: f64, duration: f64, text: &str) {
        let Ok(mut recent) = self.recent.lock() else {
            return;
        };
        recent.push((at - duration.max(0.0), at, text.to_string()));
        // Anything older than the window cannot overlap what arrives next, and
        // keeping it would grow without bound over a long meeting.
        recent.retain(|(_, end, _)| *end > at - ECHO_MEMORY_SECS);
    }

    pub fn snapshot(&self) -> Vec<(f64, f64, String)> {
        self.recent.lock().map(|r| r.clone()).unwrap_or_default()
    }
}

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

    // One clock and one window shared by both readers, so a microphone
    // utterance can be checked against what the meeting was saying at the time.
    let started = Instant::now();
    let echo = EchoWindow::default();

    spawn_reader(
        app.clone(),
        note_id.clone(),
        live_state.clone(),
        mic_rx,
        AudioSource::Mic,
        started,
        echo.clone(),
    );
    spawn_reader(
        app.clone(),
        note_id.clone(),
        live_state.clone(),
        system_rx,
        AudioSource::System,
        started,
        echo,
    );

    tokio::spawn(feed_loop(
        app.clone(),
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
    app_for_warning: AppHandle,
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
    // Frames the recogniser was too far behind to take. Counted rather than
    // ignored: this is audio the transcript will not contain, and silence about
    // it is how a meeting comes back with holes nobody can explain.
    let mut dropped: u64 = 0;
    let mut warned_behind = false;
    let mut mic_utterance = Utterance::new();
    let mut system_utterance = Utterance::new();

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
                    let close = mic_utterance.observe(frame_rms(&frame), Instant::now());
                    match mic_session.send(streaming::to_s16le(&frame)) {
                        SendOutcome::Sent => {}
                        SendOutcome::Behind => dropped += 1,
                        SendOutcome::Disconnected => break,
                    }
                    if close {
                        let _ = mic_session.finalize();
                    }
                }
            }
        }

        // System audio arrives already mono at 16 kHz, and gets no gain: it is
        // whatever the far end sent, at the level they sent it.
        let system_samples = take_system_audio_samples();
        if !system_samples.is_empty() {
            for frame in system_frames.push(&system_samples) {
                let close = system_utterance.observe(frame_rms(&frame), Instant::now());
                match system_session.send(streaming::to_s16le(&frame)) {
                    SendOutcome::Sent => {}
                    SendOutcome::Behind => dropped += 1,
                    SendOutcome::Disconnected => break,
                }
                if close {
                    let _ = system_session.finalize();
                }
            }
        }

        // Say so once, as soon as it starts happening, rather than at the end
        // when the meeting is over and nothing can be done about it.
        if dropped > 0 && !warned_behind {
            warned_behind = true;
            println!(
                "[stream] the recogniser is not keeping up — audio is being dropped and the \
                 transcript will fall behind"
            );
            let _ = app_for_warning.emit(
                "transcription-falling-behind",
                "The recogniser is not keeping up. The transcript is behind and some audio is \
                 being lost — the recording itself is unaffected.",
            );
        }
    }

    if dropped > 0 {
        println!("[stream] {dropped} frame(s) were dropped because the recogniser was behind");
    }

    // Push the tails, then drop the sessions — which closes the sockets, and
    // the writer sends the finalize frame on its way out so the last utterance
    // still comes back.
    if let Some(mut frame) = mic_frames.flush() {
        gain.apply(&mut frame);
        let _ = mic_session.send(streaming::to_s16le(&frame));
    }
    if let Some(frame) = system_frames.flush() {
        let _ = system_session.send(streaming::to_s16le(&frame));
    }

    // Give the finals a moment to come back before the readers see the socket
    // close. Without it, speech already spoken would be dropped on the floor.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    drop(mic_session);
    drop(system_session);
    live_state.is_running.store(false, Ordering::SeqCst);
}

/// Consumes one track's recognitions: partials to the UI, finals to the DB.
#[allow(clippy::too_many_arguments)]
fn spawn_reader(
    app: AppHandle,
    note_id: String,
    live_state: Arc<LiveTranscriptionState>,
    mut rx: tokio::sync::mpsc::Receiver<Recognised>,
    source: AudioSource,
    started: Instant,
    echo: EchoWindow,
) {
    tokio::spawn(async move {
        while let Some(item) = rx.recv().await {
            match item {
                Recognised::Partial {
                    text,
                    start_time,
                    end_time,
                } => {
                    if text.trim().is_empty() {
                        continue;
                    }
                    // Partials are redrawn in place and never stored: they are
                    // the recogniser's current guess, and half of them are
                    // wrong by design.
                    // On the same clock as the final that will replace it, so a
                    // draft does not jump position the moment it settles.
                    let at = started.elapsed().as_secs_f64();
                    let segment = TranscriptionSegment {
                        start_time: (at - (end_time - start_time).max(0.0)).max(0.0),
                        end_time: at,
                        text,
                        speaker: Some(speaker_for(source).to_string()),
                    };
                    let _ = app.emit(
                        "transcription-update",
                        TranscriptionUpdateEvent {
                            note_id: note_id.clone(),
                            segments: vec![segment],
                            is_final: false,
                            partial: true,
                            audio_source: source,
                        },
                    );
                }
                Recognised::Final {
                    text,
                    start_time,
                    end_time,
                } => {
                    let duration = (end_time - start_time).max(0.0);

                    match source {
                        AudioSource::System => {
                            // The meeting's own words, which the microphone may
                            // be about to repeat back.
                            echo.record(started.elapsed().as_secs_f64(), duration, &text);
                        }
                        AudioSource::Mic => {
                            // Wait for the far end's matching utterance to land
                            // before judging, then check.
                            tokio::time::sleep(ECHO_GRACE).await;
                            let at = started.elapsed().as_secs_f64() - ECHO_GRACE.as_secs_f64();
                            if is_echo_of_system(
                                &text,
                                at - duration,
                                at,
                                &echo.snapshot(),
                            ) {
                                println!(
                                    "[stream] dropped a mic utterance as an echo of the meeting audio: {text:?}"
                                );
                                // Take the draft off screen. It was shown as the
                                // user speaking and it was the room's speakers,
                                // so leaving it would attribute the far end's
                                // words to whoever is wearing the microphone.
                                let _ = app.emit(
                                    "transcription-update",
                                    TranscriptionUpdateEvent {
                                        note_id: note_id.clone(),
                                        segments: vec![],
                                        is_final: false,
                                        partial: false,
                                        audio_source: source,
                                    },
                                );
                                continue;
                            }
                        }
                    }

                    // Placed on one shared clock, not on the track's own.
                    //
                    // Each track's clock counts what its socket has been sent,
                    // and Windows loopback delivers nothing while no
                    // application is playing audio — so the system track's
                    // offset falls behind the microphone's by however much
                    // silence has passed. Over half an hour of ordinary
                    // back-and-forth that drift is minutes, and since the
                    // transcript is ordered by timestamp the far end's lines
                    // get stamped further and further into the past and pile up
                    // above, while the microphone's stay near the bottom. That
                    // happened in a real call, and it gets worse the longer the
                    // recording runs.
                    //
                    // What both clocks measure reliably is *duration*, so an
                    // utterance is placed by when it came back and extended
                    // backwards by how long it ran. Both tracks then share one
                    // frame and cannot drift apart. It is late by whatever the
                    // recogniser took, equally for both, which keeps their
                    // order right — the thing that was actually broken.
                    let placed_end = started.elapsed().as_secs_f64() - latency_allowance(source);
                    let placed_start = (placed_end - duration).max(0.0);

                    let segment = TranscriptionSegment {
                        start_time: placed_start,
                        end_time: placed_end.max(placed_start),
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
                            // Not `is_final`: that means the whole stream has
                            // ended, and the UI switches the recording
                            // indicator off when it sees it. This is one
                            // settled utterance in a meeting still running.
                            is_final: false,
                            partial: false,
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

    #[test]
    fn an_utterance_is_placed_by_when_it_arrived_less_how_long_it_ran() {
        // The two track clocks count what each socket was sent, and the system
        // track stalls whenever nothing is playing — so absolute offsets from
        // the clocks are not comparable between tracks. Arrival is.
        let w = EchoWindow::default();
        w.record(12.0, 3.0, "shall we start");
        assert_eq!(w.snapshot(), vec![(9.0, 12.0, "shall we start".to_string())]);
    }

    #[test]
    fn the_window_forgets_what_can_no_longer_overlap() {
        let w = EchoWindow::default();
        w.record(1.0, 1.0, "old news");
        w.record(ECHO_MEMORY_SECS + 5.0, 2.0, "current");
        let kept = w.snapshot();
        assert_eq!(kept.len(), 1, "a long meeting must not grow this for ever");
        assert_eq!(kept[0].2, "current");
    }

    #[test]
    fn a_negative_duration_does_not_invert_the_span() {
        // Defensive: the clock should never produce one, and an inverted span
        // would silently never match rather than failing loudly.
        let w = EchoWindow::default();
        w.record(5.0, -2.0, "odd");
        let (start, end, _) = w.snapshot()[0].clone();
        assert!(start <= end, "span inverted: {start} > {end}");
    }

    #[test]
    fn the_speakers_playing_the_far_end_back_is_recognised_as_echo() {
        // The case this whole mechanism exists for: testing on speakers, where
        // the microphone hears the meeting and reports it as the user talking.
        let w = EchoWindow::default();
        w.record(10.0, 4.0, "we should ship it on Friday");
        // The mic hears the same words over the same stretch.
        assert!(is_echo_of_system(
            "we should ship it on Friday",
            6.0,
            10.0,
            &w.snapshot()
        ));
    }

    #[test]
    fn talking_over_the_meeting_is_not_echo() {
        // The failure that matters more than a duplicate: dropping what the
        // user actually said because they spoke while the far end was talking.
        let w = EchoWindow::default();
        w.record(10.0, 4.0, "we should ship it on Friday");
        assert!(!is_echo_of_system(
            "sorry, can I stop you there",
            6.0,
            10.0,
            &w.snapshot()
        ));
    }

    #[test]
    fn nothing_is_echo_when_the_meeting_has_said_nothing() {
        let w = EchoWindow::default();
        assert!(!is_echo_of_system("just thinking aloud", 0.0, 3.0, &w.snapshot()));
    }

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    #[test]
    fn a_pause_in_speech_closes_the_utterance() {
        // The boundary that gives a transcript its shape. Without it a whole
        // meeting is one utterance: partials that are ever-growing prefixes of
        // the entire conversation, a single final at the end, and every
        // timestamp at zero. That is what a real hour-long meeting produced.
        let base = Instant::now();
        let mut u = Utterance::new();

        assert!(!u.observe(0.2, base), "speech should not close anything");
        assert!(!u.observe(0.0, at(base, 300)), "a short gap is not a boundary");
        assert!(
            u.observe(0.0, at(base, 1_000)),
            "a second of silence should close the utterance"
        );
    }

    #[test]
    fn silence_before_anything_was_said_asks_for_nothing() {
        // Otherwise a quiet meeting sends a finalize every tick and the
        // recogniser answers with an endless run of empty finals.
        let base = Instant::now();
        let mut u = Utterance::new();
        for tick in 0..50 {
            assert!(
                !u.observe(0.0, at(base, tick * 100)),
                "silence closed an utterance that never opened"
            );
        }
    }

    #[test]
    fn one_pause_closes_once_not_repeatedly() {
        // After closing, the next silence must not close again — each close is
        // a request to the recogniser and repeating it wastes the round trip
        // and litters the transcript with empty finals.
        let base = Instant::now();
        let mut u = Utterance::new();
        u.observe(0.2, base);
        assert!(u.observe(0.0, at(base, 1_000)));
        assert!(!u.observe(0.0, at(base, 2_000)));
        assert!(!u.observe(0.0, at(base, 5_000)));
    }

    #[test]
    fn someone_who_never_pauses_is_still_broken_up() {
        // The backstop. Twenty minutes of unbroken speech is one unreadable
        // segment, and it grows the recogniser's working state until the
        // transcript falls minutes behind — which is what happened after about
        // half an hour of a real meeting.
        let base = Instant::now();
        let mut u = Utterance::new();
        let mut closes = 0;
        for tick in 0..600 {
            if u.observe(0.3, at(base, tick * 100)) {
                closes += 1;
            }
        }
        assert!(
            closes >= 2,
            "a minute of continuous speech produced {closes} boundaries"
        );
    }

    #[test]
    fn speech_resumes_a_new_utterance_after_a_close() {
        let base = Instant::now();
        let mut u = Utterance::new();
        u.observe(0.2, base);
        assert!(u.observe(0.0, at(base, 1_000)));

        assert!(!u.observe(0.2, at(base, 2_000)), "new speech should not close");
        assert!(
            u.observe(0.0, at(base, 3_100)),
            "the next pause should close the new utterance"
        );
    }

    #[test]
    fn rms_is_zero_for_no_audio_and_positive_for_some() {
        assert_eq!(frame_rms(&[]), 0.0);
        assert_eq!(frame_rms(&[0.0; 100]), 0.0);
        assert!(frame_rms(&[0.5; 100]) > 0.4);
    }

    #[test]
    fn the_two_tracks_are_placed_on_one_clock() {
        // The failure this fixes, from a real call: each track was timestamped
        // by what its own socket had been sent, and Windows loopback sends
        // nothing while nothing is playing. Half an hour in, the far end's
        // lines were stamped minutes into the past and piled up above the
        // microphone's, which sat at the bottom.
        //
        // Placing by arrival less duration puts both on the same frame. The
        // only per-track adjustment is the microphone's echo grace, which is a
        // delay this code adds rather than anything about when words were said.
        assert_eq!(latency_allowance(AudioSource::System), 0.0);
        assert_eq!(
            latency_allowance(AudioSource::Mic),
            ECHO_GRACE.as_secs_f64(),
            "the mic's own hold-back must be removed, or it reads as later speech"
        );
    }

    #[test]
    fn silence_on_one_track_cannot_move_it_relative_to_the_other() {
        // The property that matters. Both tracks are placed from the same
        // `Instant`, so however long one has been quiet its next utterance
        // lands where it actually arrived — the drift has nowhere to
        // accumulate.
        let started = Instant::now();
        let after_quiet = started + Duration::from_secs(1_800);

        let system_at = after_quiet.duration_since(started).as_secs_f64()
            - latency_allowance(AudioSource::System);
        let mic_at = after_quiet.duration_since(started).as_secs_f64()
            - latency_allowance(AudioSource::Mic);

        assert!(
            (system_at - mic_at).abs() <= ECHO_GRACE.as_secs_f64() + 0.001,
            "half an hour of silence moved the tracks {} seconds apart",
            (system_at - mic_at).abs()
        );
    }

    #[test]
    fn an_utterance_never_starts_before_the_recording_did() {
        // An utterance that ran longer than the session has — possible when a
        // recogniser reports a duration from before this session began —
        // would otherwise be placed at a negative time and sort above
        // everything for ever.
        let placed_end: f64 = 2.0;
        let duration: f64 = 30.0;
        let placed_start = (placed_end - duration).max(0.0);
        assert_eq!(placed_start, 0.0);
    }

}
