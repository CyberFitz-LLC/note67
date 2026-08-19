//! Live transcription by a streaming recogniser.
//!
//! The third backend. `Local` is live but limited to on-device Whisper;
//! `Remote` is stronger but only works on a finished recording. This is live
//! *and* stronger — at a privacy cost higher than either, because raw audio
//! leaves the machine continuously while a meeting is in progress rather than
//! one file being uploaded after the fact. See `docs/exochain/DECISIONS.md`.
//!
//! **Two sockets, not one.** Live capture keeps the microphone and the system
//! output as separate tracks, and that separation is the only speaker
//! attribution the app has without a diarizer: mic is "You", system is
//! "Others". Mixing them into one stream to save a connection would throw that
//! away — and this recogniser does not diarize, so nothing would give it back.

use serde::Deserialize;

/// Chunk length on the wire. The reference client paces ~100 ms frames in real
/// time, and this is a streaming recogniser rather than a file endpoint: firing
/// audio as fast as the socket accepts is not the tested path.
pub const CHUNK_MS: usize = 100;
pub const SAMPLE_RATE: usize = 16_000;

/// Samples per frame, one channel.
pub const CHUNK_SAMPLES: usize = SAMPLE_RATE * CHUNK_MS / 1000;

/// What the service sends back.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    /// Sent once on connect.
    Ready,
    Transcript {
        #[serde(default)]
        text: String,
        #[serde(default)]
        is_final: bool,
    },
    /// Anything this client has not been taught. Ignored rather than treated as
    /// an error: a new event type is far more likely than a broken service, and
    /// tearing down a live meeting's connection over one would lose audio that
    /// cannot be recovered.
    #[serde(other)]
    Unknown,
}

/// Parse a text frame.
///
/// A frame that will not parse is `Unknown` rather than an error, for the same
/// reason: nothing about a malformed status message justifies dropping the
/// stream a meeting is being recorded into.
pub fn parse_event(frame: &str) -> ServerEvent {
    serde_json::from_str(frame).unwrap_or(ServerEvent::Unknown)
}

/// The frame that ends an utterance. The final arrives after it, so the reader
/// has to keep going briefly rather than closing on send.
pub fn finalize_frame() -> String {
    r#"{"type":"reset","finalize":true}"#.to_string()
}

/// Convert captured audio to what the wire wants: 16 kHz mono s16le.
///
/// Run this *after* `normalize_peak`, so the recogniser hears the same levels
/// Whisper would have. Values are clamped before scaling because a normalised
/// buffer can still exceed 1.0 slightly, and wrapping that into i16 turns a
/// loud syllable into a burst of noise.
pub fn to_s16le(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        let v = (clamped * i16::MAX as f32) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Where a track has got to, in seconds of audio sent.
///
/// The service's transcript events carry `text` and `is_final` and **no timing
/// at all** — verified against the running service, not assumed. So the only
/// clock is how much audio this socket has been given, which is why each track
/// keeps its own: two sockets are fed independently and their positions drift
/// apart the moment one of them is gated for silence.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TrackClock {
    pub samples_sent: usize,
}

impl TrackClock {
    pub fn advance(&mut self, samples: usize) {
        self.samples_sent += samples;
    }

    pub fn seconds(&self) -> f64 {
        self.samples_sent as f64 / SAMPLE_RATE as f64
    }

    /// The span a final covers: from where the last one ended to here.
    pub fn span_since(&self, previous: &TrackClock) -> (f64, f64) {
        (previous.seconds(), self.seconds())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ready_frame_parses() {
        assert_eq!(parse_event(r#"{"type":"ready"}"#), ServerEvent::Ready);
    }

    #[test]
    fn a_partial_and_a_final_are_distinguished() {
        // The whole UI behaviour rests on this: partials are redrawn, finals
        // are persisted.
        assert_eq!(
            parse_event(r#"{"type":"transcript","text":"hello","is_final":false}"#),
            ServerEvent::Transcript { text: "hello".into(), is_final: false }
        );
        assert_eq!(
            parse_event(r#"{"type":"transcript","text":"hello","is_final":true}"#),
            ServerEvent::Transcript { text: "hello".into(), is_final: true }
        );
    }

    #[test]
    fn the_real_final_frame_parses() {
        // Captured from the running service: it carries a `finalize` field the
        // brief did not mention, and an unknown field must not break parsing.
        assert_eq!(
            parse_event(r#"{"type":"transcript","text":"","is_final":true,"finalize":true}"#),
            ServerEvent::Transcript { text: String::new(), is_final: true }
        );
    }

    #[test]
    fn an_unknown_event_is_ignored_rather_than_fatal() {
        // Tearing down the connection a live meeting is recording into, over a
        // status message, would lose audio nobody can get back.
        assert_eq!(parse_event(r#"{"type":"something_new"}"#), ServerEvent::Unknown);
        assert_eq!(parse_event("not json at all"), ServerEvent::Unknown);
        assert_eq!(parse_event(""), ServerEvent::Unknown);
    }

    #[test]
    fn silence_converts_to_silence() {
        assert_eq!(to_s16le(&[0.0, 0.0]), vec![0, 0, 0, 0]);
    }

    #[test]
    fn conversion_is_little_endian_two_bytes_per_sample() {
        assert_eq!(to_s16le(&[1.0]).len(), 2);
        // 1.0 -> i16::MAX = 0x7FFF, low byte first.
        assert_eq!(to_s16le(&[1.0]), vec![0xFF, 0x7F]);
    }

    #[test]
    fn out_of_range_samples_clamp_instead_of_wrapping() {
        // A normalised buffer can still exceed 1.0, and wrapping would turn a
        // loud syllable into a burst of noise the recogniser reads as garbage.
        assert_eq!(to_s16le(&[2.0]), to_s16le(&[1.0]));
        assert_eq!(to_s16le(&[-2.0]), to_s16le(&[-1.0]));
    }

    #[test]
    fn a_chunk_is_a_tenth_of_a_second() {
        assert_eq!(CHUNK_SAMPLES, 1600);
        assert_eq!(to_s16le(&vec![0.0; CHUNK_SAMPLES]).len(), 3200);
    }

    #[test]
    fn a_clock_counts_the_audio_it_was_given() {
        let mut c = TrackClock::default();
        c.advance(SAMPLE_RATE);
        assert_eq!(c.seconds(), 1.0);
        c.advance(SAMPLE_RATE / 2);
        assert_eq!(c.seconds(), 1.5);
    }

    #[test]
    fn a_span_runs_from_the_previous_final_to_now() {
        let mut previous = TrackClock::default();
        previous.advance(SAMPLE_RATE * 2);
        let mut now = previous;
        now.advance(SAMPLE_RATE * 3);
        assert_eq!(now.span_since(&previous), (2.0, 5.0));
    }

    #[test]
    fn two_tracks_keep_independent_clocks() {
        // The reason a clock is per-track rather than global: the mic is gated
        // for silence independently of system audio, so the two positions drift
        // apart immediately. One shared clock would timestamp every segment on
        // whichever track happened to be busier.
        let mut mic = TrackClock::default();
        let mut system = TrackClock::default();
        mic.advance(SAMPLE_RATE);
        system.advance(SAMPLE_RATE * 4);
        assert_ne!(mic.seconds(), system.seconds());
    }
}
