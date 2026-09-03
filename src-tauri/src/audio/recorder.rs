use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{Sample, SampleFormat};
use hound::{WavSpec, WavWriter};
use serde::{Deserialize, Serialize};

use crate::audio::AudioError;

/// Recording phase for pause/resume functionality
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum RecordingPhase {
    Idle = 0,
    Recording = 1,
    Paused = 2,
}

impl RecordingPhase {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => RecordingPhase::Recording,
            2 => RecordingPhase::Paused,
            _ => RecordingPhase::Idle,
        }
    }
}

/// Shared state that can be accessed across threads
pub struct RecordingState {
    pub is_recording: AtomicBool,
    pub audio_level: AtomicU32,
    pub output_path: std::sync::Mutex<Option<PathBuf>>,
    /// Buffer for live transcription - stores raw f32 samples
    pub audio_buffer: std::sync::Mutex<Vec<f32>>,
    /// Sample rate of the recorded audio (set when recording starts)
    pub sample_rate: AtomicU32,
    /// Number of channels (set when recording starts)
    pub channels: AtomicU32,
    /// Input device the user pinned, by cpal device name. `None` follows the
    /// system default. Read when a segment starts, so changing it mid-recording
    /// takes effect on the next segment rather than cutting the current one.
    pub preferred_input_device: std::sync::Mutex<Option<String>>,
    /// The device the current stream actually opened.
    ///
    /// Distinct from the preference: a pinned device that has gone away falls
    /// back to the default silently, so the preference alone does not say what
    /// is being recorded. Without this the only way to tell which device a
    /// meter is reading is to guess.
    pub opened_input_device: std::sync::Mutex<Option<String>>,

    // === Pause/Resume/Continue fields ===
    /// Current recording phase (Idle, Recording, Paused)
    pub phase: AtomicU8,
    /// Current segment index (0-based)
    pub current_segment_index: AtomicU32,
    /// Start offset in milliseconds from the note start (for continued recordings)
    pub segment_start_offset_ms: AtomicI64,
    /// When the current segment started recording (for duration calculation)
    pub segment_start_time: std::sync::Mutex<Option<Instant>>,
    /// Current note ID being recorded
    pub current_note_id: std::sync::Mutex<Option<String>>,
    /// Current segment ID in database (for updating duration)
    pub current_segment_db_id: AtomicI64,
    /// Set once the recording thread has dropped the stream and finalized the
    /// WAV. Anything that reads the file after stopping must wait for this:
    /// the thread owns the writer, and hound only writes the header (and hence
    /// the sample count) on finalize.
    pub file_finalized: AtomicBool,
}

impl RecordingState {
    pub fn new() -> Self {
        Self {
            is_recording: AtomicBool::new(false),
            audio_level: AtomicU32::new(0),
            output_path: std::sync::Mutex::new(None),
            audio_buffer: std::sync::Mutex::new(Vec::new()),
            sample_rate: AtomicU32::new(0),
            channels: AtomicU32::new(0),
            preferred_input_device: std::sync::Mutex::new(None),
            opened_input_device: std::sync::Mutex::new(None),
            // Pause/Resume/Continue fields
            phase: AtomicU8::new(RecordingPhase::Idle as u8),
            current_segment_index: AtomicU32::new(0),
            segment_start_offset_ms: AtomicI64::new(0),
            segment_start_time: std::sync::Mutex::new(None),
            current_note_id: std::sync::Mutex::new(None),
            current_segment_db_id: AtomicI64::new(0),
            // Nothing has been recorded yet, so there is nothing to wait for.
            file_finalized: AtomicBool::new(true),
        }
    }

    /// Get the pinned input device name, if any.
    ///
    /// A poisoned lock falls back to the system default rather than failing the
    /// recording — losing the device preference is a far smaller problem than
    /// losing the meeting.
    pub fn get_preferred_input_device(&self) -> Option<String> {
        self.preferred_input_device
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
    }

    /// Pin an input device by name. `None` follows the system default.
    pub fn set_preferred_input_device(&self, name: Option<String>) -> Result<(), AudioError> {
        let mut guard = self
            .preferred_input_device
            .lock()
            .map_err(|_| AudioError::LockError)?;
        // Normalise "" to None so the resolver has one representation of
        // "follow the system" to reason about.
        *guard = name.filter(|n| !n.trim().is_empty());
        Ok(())
    }

    /// Get the current recording phase
    pub fn get_phase(&self) -> RecordingPhase {
        RecordingPhase::from_u8(self.phase.load(Ordering::SeqCst))
    }

    /// Set the recording phase
    pub fn set_phase(&self, phase: RecordingPhase) {
        self.phase.store(phase as u8, Ordering::SeqCst);
    }

    /// Get the elapsed time since segment start in milliseconds
    pub fn get_segment_elapsed_ms(&self) -> i64 {
        if let Ok(start_time) = self.segment_start_time.lock()
            && let Some(start) = *start_time
        {
            return start.elapsed().as_millis() as i64;
        }
        0
    }

    /// Reset state for a new recording session
    pub fn reset_for_new_session(&self) {
        self.current_segment_index.store(0, Ordering::SeqCst);
        self.segment_start_offset_ms.store(0, Ordering::SeqCst);
        self.current_segment_db_id.store(0, Ordering::SeqCst);
        if let Ok(mut start_time) = self.segment_start_time.lock() {
            *start_time = None;
        }
        if let Ok(mut note_id) = self.current_note_id.lock() {
            *note_id = None;
        }
    }

    /// Take all samples from the buffer (clears the buffer)
    pub fn take_audio_buffer(&self) -> Vec<f32> {
        match self.audio_buffer.lock() { Ok(mut buffer) => {
            std::mem::take(&mut *buffer)
        } _ => {
            Vec::new()
        }}
    }

    /// Get the current buffer length without clearing
    #[allow(dead_code)]
    pub fn buffer_len(&self) -> usize {
        match self.audio_buffer.lock() { Ok(buffer) => {
            buffer.len()
        } _ => {
            0
        }}
    }
}

impl Default for RecordingState {
    fn default() -> Self {
        Self::new()
    }
}

/// Start recording audio to the specified path
/// Returns immediately, recording happens in a background thread
pub fn start_recording(state: Arc<RecordingState>, output_path: PathBuf) -> Result<(), AudioError> {
    let current_phase = state.get_phase();
    if current_phase == RecordingPhase::Recording {
        return Err(AudioError::AlreadyRecording);
    }

    // Store output path
    {
        let mut path = state.output_path.lock().map_err(|_| AudioError::LockError)?;
        *path = Some(output_path.clone());
    }

    // Set segment start time
    {
        let mut start_time = state.segment_start_time.lock().map_err(|_| AudioError::LockError)?;
        *start_time = Some(Instant::now());
    }

    state.is_recording.store(true, Ordering::SeqCst);
    state.set_phase(RecordingPhase::Recording);
    state.file_finalized.store(false, Ordering::SeqCst);

    let state_clone = state.clone();

    // Spawn recording thread
    thread::spawn(move || {
        if let Err(e) = run_recording(state_clone, output_path) {
            eprintln!("Recording error: {}", e);
        }
    });

    Ok(())
}

/// Pause recording - stops the current segment but keeps state for resume
pub fn pause_recording(state: &RecordingState) -> Result<i64, AudioError> {
    let current_phase = state.get_phase();
    if current_phase != RecordingPhase::Recording {
        return Err(AudioError::NotRecording);
    }

    // Calculate duration before stopping
    let duration_ms = state.get_segment_elapsed_ms();

    // Stop the recording thread
    state.is_recording.store(false, Ordering::SeqCst);
    state.audio_level.store(0, Ordering::SeqCst);
    state.set_phase(RecordingPhase::Paused);

    // A paused segment's WAV is a finished file that later gets merged and
    // retranscribed, so wait for it to close here too.
    await_file_finalized(state, std::time::Duration::from_secs(5));

    Ok(duration_ms)
}

/// Resume recording after pause - starts a new segment
pub fn resume_recording(state: Arc<RecordingState>, output_path: PathBuf) -> Result<(), AudioError> {
    let current_phase = state.get_phase();
    if current_phase != RecordingPhase::Paused {
        return Err(AudioError::NotPaused);
    }

    // Increment segment index
    let new_index = state.current_segment_index.fetch_add(1, Ordering::SeqCst) + 1;
    state.current_segment_index.store(new_index, Ordering::SeqCst);

    // Start recording with the new path
    start_recording(state, output_path)
}

/// Block until the recording thread has finalized the WAV, or `timeout` passes.
///
/// The thread owns the writer and only finalizes after it observes
/// `is_recording == false`. Callers that immediately read the file — the mixer
/// that builds the playback track — otherwise see a WAV whose header still says
/// zero samples, and silently mix in nothing. Returns whether it finalized.
fn await_file_finalized(state: &RecordingState, timeout: std::time::Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while !state.file_finalized.load(Ordering::SeqCst) {
        if Instant::now() >= deadline {
            eprintln!(
                "Timed out waiting for the recording file to finalize; playback audio may be incomplete"
            );
            return false;
        }
        thread::sleep(std::time::Duration::from_millis(5));
    }
    true
}

/// Stop recording completely - resets all state
pub fn stop_recording(state: &RecordingState) -> Result<Option<PathBuf>, AudioError> {
    state.is_recording.store(false, Ordering::SeqCst);
    state.audio_level.store(0, Ordering::SeqCst);
    state.set_phase(RecordingPhase::Idle);

    // The caller is about to read this file (to merge the playback track), so
    // it must not be handed a path to a WAV that has not been closed yet.
    await_file_finalized(state, std::time::Duration::from_secs(5));

    // Reset segment tracking
    state.reset_for_new_session();

    let path = state.output_path.lock().map_err(|_| AudioError::LockError)?;
    Ok(path.clone())
}

/// Stop recording but preserve state for continue (used when ending a note that can be continued)
#[allow(dead_code)]
pub fn stop_recording_preserving_state(state: &RecordingState) -> Result<(Option<PathBuf>, i64), AudioError> {
    // Calculate duration before stopping
    let duration_ms = state.get_segment_elapsed_ms();

    state.is_recording.store(false, Ordering::SeqCst);
    state.audio_level.store(0, Ordering::SeqCst);
    state.set_phase(RecordingPhase::Idle);

    let path = state.output_path.lock().map_err(|_| AudioError::LockError)?;
    Ok((path.clone(), duration_ms))
}

fn run_recording(state: Arc<RecordingState>, output_path: PathBuf) -> Result<(), AudioError> {
    let preferred = state.get_preferred_input_device();
    let device = crate::audio::devices::open_input_device(preferred.as_deref())?;
    if let Ok(mut opened) = state.opened_input_device.lock() {
        *opened = device.name().ok();
    }

    let config = device.default_input_config()?;
    let sample_rate = config.sample_rate().0;
    let channels = config.channels();

    // Store sample rate and channels for live transcription
    state.sample_rate.store(sample_rate, Ordering::SeqCst);
    state.channels.store(channels as u32, Ordering::SeqCst);

    // Clear the audio buffer at start
    if let Ok(mut buffer) = state.audio_buffer.lock() {
        buffer.clear();
    }

    let spec = WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let writer = WavWriter::create(&output_path, spec)?;
    let writer = Arc::new(std::sync::Mutex::new(Some(writer)));

    let state_for_callback = state.clone();
    let writer_clone = writer.clone();

    let err_fn = |err| eprintln!("Audio stream error: {}", err);

    let stream = match config.sample_format() {
        SampleFormat::F32 => device.build_input_stream(
            &config.into(),
            move |data: &[f32], _| {
                process_audio(data, &state_for_callback, &writer_clone);
            },
            err_fn,
            None,
        )?,
        SampleFormat::I16 => {
            let state_for_callback = state.clone();
            let writer_clone = writer.clone();
            device.build_input_stream(
                &config.into(),
                move |data: &[i16], _| {
                    let float_data: Vec<f32> = data.iter().map(|&s| s.to_float_sample()).collect();
                    process_audio(&float_data, &state_for_callback, &writer_clone);
                },
                err_fn,
                None,
            )?
        }
        SampleFormat::U16 => {
            let state_for_callback = state.clone();
            let writer_clone = writer.clone();
            device.build_input_stream(
                &config.into(),
                move |data: &[u16], _| {
                    let float_data: Vec<f32> = data.iter().map(|&s| s.to_float_sample()).collect();
                    process_audio(&float_data, &state_for_callback, &writer_clone);
                },
                err_fn,
                None,
            )?
        }
        _ => return Err(AudioError::UnsupportedFormat),
    };

    stream.play()?;

    // Keep thread alive while recording. Polled rather than signalled; the
    // interval is the floor on how long a stop takes, since the caller waits
    // for the finalize below.
    while state.is_recording.load(Ordering::SeqCst) {
        thread::sleep(std::time::Duration::from_millis(20));
    }

    // Finalize the WAV file
    drop(stream);
    if let Ok(mut guard) = writer.lock()
        && let Some(w) = guard.take()
    {
        let _ = w.finalize();
    }
    state.file_finalized.store(true, Ordering::SeqCst);

    Ok(())
}

/// How many samples either capture buffer may hold.
///
/// Thirty seconds at 48 kHz stereo — comfortably more than any consumer needs
/// between drains (whisper takes every three seconds, the streaming feed every
/// hundred milliseconds) and bounded at about eleven megabytes rather than
/// unbounded.
pub const MAX_BUFFERED_SAMPLES: usize = 48_000 * 2 * 30;

/// Drop the oldest samples so a buffer stays near `limit`.
///
/// Only once it has drifted a quarter past the limit, not on every call.
/// Draining from the front moves the whole remaining buffer, and this runs
/// inside the capture callback — trimming on every callback would put an
/// eleven-megabyte memmove on the audio thread hundreds of times a second,
/// which is the same kind of fault as the leak it is fixing. With the slack it
/// happens roughly once every seven seconds instead.
///
/// Drains rather than clears: a brief stall should cost the stalled consumer
/// nothing, and only a sustained one should lose audio.
pub fn trim_to_recent(buffer: &mut Vec<f32>, limit: usize) {
    let slack = limit / 4;
    if buffer.len() > limit + slack {
        let excess = buffer.len() - limit;
        buffer.drain(..excess);
    }
}

/// The most either buffer can hold, slack included.
pub fn buffer_ceiling(limit: usize) -> usize {
    limit + limit / 4
}

fn process_audio(
    data: &[f32],
    state: &Arc<RecordingState>,
    writer: &Arc<std::sync::Mutex<Option<WavWriter<std::io::BufWriter<std::fs::File>>>>>,
) {
    if !state.is_recording.load(Ordering::SeqCst) {
        return;
    }

    // Calculate RMS audio level
    let sum: f32 = data.iter().map(|s| s * s).sum();
    let rms = (sum / data.len() as f32).sqrt();
    state.audio_level.store(rms.to_bits(), Ordering::SeqCst);

    // Copy samples to buffer for live transcription, bounded.
    //
    // This buffer is filled by the capture callback and emptied by whoever is
    // transcribing. When nothing is — no model loaded, live transcription not
    // started, or the streaming feed loop having stopped because its socket
    // died — it used to grow for as long as the recording ran. At 48 kHz
    // stereo that is over a gigabyte an hour of memory nobody would ever read,
    // and on a machine also carrying a video call the result is a machine that
    // starts stuttering about ninety minutes in. That happened, in a real
    // two-hour meeting, and the recording is not what failed — the rest of the
    // computer did.
    //
    // Bounded, and the oldest goes first: a consumer that has stalled wants the
    // current audio, not a backlog it can never catch up on.
    if let Ok(mut buffer) = state.audio_buffer.lock() {
        buffer.extend_from_slice(data);
        trim_to_recent(&mut buffer, MAX_BUFFERED_SAMPLES);
    }

    // Write to WAV file
    if let Ok(mut guard) = writer.lock()
        && let Some(ref mut w) = *guard
    {
        for &sample in data {
            let sample_i16 = (sample * i16::MAX as f32) as i16;
            let _ = w.write_sample(sample_i16);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_state_pins_no_input_device() {
        assert_eq!(RecordingState::new().get_preferred_input_device(), None);
    }

    #[test]
    fn the_pinned_input_device_round_trips() {
        let state = RecordingState::new();
        state
            .set_preferred_input_device(Some("Blue Yeti".to_string()))
            .unwrap();
        assert_eq!(
            state.get_preferred_input_device(),
            Some("Blue Yeti".to_string())
        );
    }

    #[test]
    fn a_blank_device_name_means_follow_the_system_default() {
        // The settings store round-trips strings, so clearing the preference in
        // the UI can arrive here as "" rather than as None.
        let state = RecordingState::new();
        state
            .set_preferred_input_device(Some("Blue Yeti".to_string()))
            .unwrap();

        state.set_preferred_input_device(Some("".to_string())).unwrap();
        assert_eq!(state.get_preferred_input_device(), None);

        state
            .set_preferred_input_device(Some("Blue Yeti".to_string()))
            .unwrap();
        state.set_preferred_input_device(Some("  ".to_string())).unwrap();
        assert_eq!(state.get_preferred_input_device(), None);

        state
            .set_preferred_input_device(Some("Blue Yeti".to_string()))
            .unwrap();
        state.set_preferred_input_device(None).unwrap();
        assert_eq!(state.get_preferred_input_device(), None);
    }

    #[test]
    fn starting_a_new_session_keeps_the_pinned_device() {
        // Segment bookkeeping resets between recordings; the user's microphone
        // choice must not.
        let state = RecordingState::new();
        state
            .set_preferred_input_device(Some("Blue Yeti".to_string()))
            .unwrap();

        state.reset_for_new_session();

        assert_eq!(
            state.get_preferred_input_device(),
            Some("Blue Yeti".to_string())
        );
    }

    #[test]
    fn fresh_state_does_not_wait_for_a_finalize_that_will_never_come() {
        // Nothing was recorded, so there is no thread to finalize anything.
        // Waiting here would stall every stop on a note that never recorded.
        let state = RecordingState::new();
        let started = Instant::now();
        assert!(await_file_finalized(&state, std::time::Duration::from_secs(5)));
        assert!(
            started.elapsed() < std::time::Duration::from_millis(100),
            "should have returned immediately"
        );
    }

    #[test]
    fn waits_until_the_recording_thread_signals() {
        let state = Arc::new(RecordingState::new());
        state.file_finalized.store(false, Ordering::SeqCst);

        let signaller = state.clone();
        thread::spawn(move || {
            thread::sleep(std::time::Duration::from_millis(60));
            signaller.file_finalized.store(true, Ordering::SeqCst);
        });

        let started = Instant::now();
        assert!(await_file_finalized(&state, std::time::Duration::from_secs(5)));
        assert!(
            started.elapsed() >= std::time::Duration::from_millis(50),
            "returned before the writer signalled"
        );
    }

    #[test]
    fn gives_up_rather_than_hanging_when_no_signal_arrives() {
        let state = RecordingState::new();
        state.file_finalized.store(false, Ordering::SeqCst);

        let started = Instant::now();
        assert!(!await_file_finalized(
            &state,
            std::time::Duration::from_millis(80)
        ));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }
    #[test]
    fn a_buffer_nobody_is_reading_stops_growing() {
        // The bug this exists for, from a real two-hour meeting: only the
        // streaming feed loop drains this buffer, it stops when its socket
        // dies, and the capture callback carries on. At 48 kHz stereo that is
        // over a gigabyte an hour of memory nothing will ever read — and the
        // machine, which was also carrying a video call, began stuttering
        // around ninety minutes in.
        // Two hours of capture, fed a second at a time. A second per iteration
        // rather than a callback's worth keeps the test quick while still
        // driving the buffer far past its limit — the invariant is about size,
        // not about chunking.
        let mut buffer: Vec<f32> = Vec::new();
        let one_second = vec![0.1f32; 48_000 * 2];
        for _ in 0..7_200 {
            buffer.extend_from_slice(&one_second);
            trim_to_recent(&mut buffer, MAX_BUFFERED_SAMPLES);
        }
        assert!(
            buffer.len() <= buffer_ceiling(MAX_BUFFERED_SAMPLES),
            "two hours of unread capture grew to {} samples",
            buffer.len()
        );
    }

    #[test]
    fn trimming_is_rare_enough_for_an_audio_callback() {
        // Draining moves the whole buffer, so doing it on every callback would
        // put an eleven-megabyte memmove on the audio thread hundreds of times
        // a second. Counted, because "it is bounded" and "it is cheap" are
        // different claims and this needs both.
        let mut buffer: Vec<f32> = vec![0.0; MAX_BUFFERED_SAMPLES];
        let mut trims = 0;
        let chunk = vec![0.1f32; 1024];
        for _ in 0..2_000 {
            let before = buffer.len();
            buffer.extend_from_slice(&chunk);
            trim_to_recent(&mut buffer, MAX_BUFFERED_SAMPLES);
            if buffer.len() < before + chunk.len() {
                trims += 1;
            }
        }
        assert!(
            trims <= 2,
            "trimmed {trims} times in 2000 callbacks; that belongs off the audio thread"
        );
    }

    #[test]
    fn the_bound_is_about_eleven_megabytes_not_gigabytes() {
        // Stated in the units that matter. Thirty seconds is generous next to
        // the three-second whisper cycle and the hundred-millisecond streaming
        // one, and it is what makes the failure survivable rather than fatal.
        let bytes = buffer_ceiling(MAX_BUFFERED_SAMPLES) * std::mem::size_of::<f32>();
        assert!(bytes < 20 * 1024 * 1024, "{bytes} bytes is too much to hold");
        assert!(bytes > 4 * 1024 * 1024, "{bytes} bytes is too little to be useful");
    }

    #[test]
    fn a_brief_stall_costs_the_consumer_nothing() {
        // Trimming drops the oldest rather than clearing, so a consumer that
        // paused for a second still gets everything it missed.
        let mut buffer: Vec<f32> = vec![0.5; 48_000];
        trim_to_recent(&mut buffer, MAX_BUFFERED_SAMPLES);
        assert_eq!(buffer.len(), 48_000, "a second of audio was discarded");
    }

    #[test]
    fn trimming_keeps_the_newest_audio() {
        // A stalled consumer wants the current conversation, not a backlog it
        // can never catch up on.
        let mut buffer: Vec<f32> = Vec::new();
        buffer.extend(std::iter::repeat_n(1.0, MAX_BUFFERED_SAMPLES));
        buffer.extend(std::iter::repeat_n(2.0, MAX_BUFFERED_SAMPLES / 2));
        trim_to_recent(&mut buffer, MAX_BUFFERED_SAMPLES);

        assert_eq!(buffer.len(), MAX_BUFFERED_SAMPLES);
        assert_eq!(*buffer.last().unwrap(), 2.0, "the newest audio was dropped");
    }

}
