//! Measuring what a device is actually hearing.
//!
//! The recorder has always tracked a level, but only for the microphone, and
//! only while recording. That is why changing the system-audio device mid-call
//! appeared to do nothing: the meter was never measuring that track, so it
//! could not have moved.
//!
//! These are the numbers a device test needs. Kept separate from capture so the
//! interpretation — silent, quiet, healthy, clipping — can be tested without an
//! audio device, which is the part that decides what the user is told.

use std::sync::atomic::{AtomicU32, Ordering};

/// Root mean square of a block of samples: how loud it is on average.
///
/// An empty block is silence rather than an error. Capture callbacks do
/// occasionally deliver nothing, and a device test that errored on it would
/// blame the user's setup for a normal event.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len() as f32).sqrt()
}

/// The loudest single sample, which is what reveals clipping.
///
/// RMS alone hides it: a track that clips on peaks can sit at a comfortable
/// average and sound fine on the meter while the transcript degrades.
pub fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |max, s| max.max(s.abs()))
}

/// Amplitude as decibels below full scale.
///
/// Meters are read in dB because hearing is logarithmic: half the amplitude is
/// −6 dB, not "half as loud". A linear bar spends most of its length on
/// differences nobody can hear and squeezes the useful range into a sliver.
pub fn dbfs(amplitude: f32) -> f32 {
    if amplitude <= 0.0 {
        // Zero has no logarithm. −100 dBFS is far below anything audible and
        // keeps the meter arithmetic total.
        return -100.0;
    }
    20.0 * amplitude.log10()
}

/// What to tell someone about a track they are testing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing at all. The device is wrong, muted, or routed elsewhere.
    Silent,
    /// Signal present but low. Usable; worth mentioning.
    Quiet,
    Healthy,
    /// Peaks at or beyond full scale. Whisper transcribes distortion badly, and
    /// this is invisible on an average-only meter.
    Clipping,
}

/// Below this, treat a track as carrying nothing.
///
/// Not zero: a live input is never exactly silent. Real rooms and real
/// interfaces sit around −60 dBFS with nobody speaking, so a threshold of zero
/// would call an empty room "healthy" and defeat the whole test.
pub const SILENCE_DBFS: f32 = -55.0;
/// Below this, present but quieter than it should be.
pub const QUIET_DBFS: f32 = -35.0;
/// At or above this, peaks are effectively at full scale.
pub const CLIPPING_DBFS: f32 = -0.5;

pub fn verdict(rms_amplitude: f32, peak_amplitude: f32) -> Verdict {
    // Clipping is checked first and from the peak, because a clipping track can
    // have a perfectly ordinary average — which is exactly why it goes
    // unnoticed.
    if dbfs(peak_amplitude) >= CLIPPING_DBFS {
        return Verdict::Clipping;
    }
    match dbfs(rms_amplitude) {
        db if db < SILENCE_DBFS => Verdict::Silent,
        db if db < QUIET_DBFS => Verdict::Quiet,
        _ => Verdict::Healthy,
    }
}

impl Verdict {
    pub fn as_str(&self) -> &'static str {
        match self {
            Verdict::Silent => "silent",
            Verdict::Quiet => "quiet",
            Verdict::Healthy => "healthy",
            Verdict::Clipping => "clipping",
        }
    }
}

/// A level that a capture thread writes and the UI reads.
///
/// Peak is held rather than replaced, so a brief clip survives long enough to
/// be seen. A meter sampled a few times a second otherwise misses exactly the
/// transients that matter.
#[derive(Debug, Default)]
pub struct LevelMeter {
    rms: AtomicU32,
    peak_hold: AtomicU32,
}

impl LevelMeter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn observe(&self, samples: &[f32]) {
        self.rms.store(rms(samples).to_bits(), Ordering::Relaxed);
        let p = peak(samples);
        let held = f32::from_bits(self.peak_hold.load(Ordering::Relaxed));
        if p > held {
            self.peak_hold.store(p.to_bits(), Ordering::Relaxed);
        }
    }

    pub fn rms(&self) -> f32 {
        f32::from_bits(self.rms.load(Ordering::Relaxed))
    }

    pub fn peak(&self) -> f32 {
        f32::from_bits(self.peak_hold.load(Ordering::Relaxed))
    }

    /// Clear both, for the start of a test.
    pub fn reset(&self) {
        self.rms.store(0f32.to_bits(), Ordering::Relaxed);
        self.peak_hold.store(0f32.to_bits(), Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_measures_zero() {
        assert_eq!(rms(&[0.0; 64]), 0.0);
        assert_eq!(peak(&[0.0; 64]), 0.0);
    }

    #[test]
    fn an_empty_block_is_silence_not_an_error() {
        // Capture callbacks do deliver empty blocks. Erroring would blame the
        // user's setup for a normal event.
        assert_eq!(rms(&[]), 0.0);
        assert_eq!(peak(&[]), 0.0);
    }

    #[test]
    fn a_full_scale_tone_measures_full_scale() {
        let square = [1.0f32, -1.0, 1.0, -1.0];
        assert!((rms(&square) - 1.0).abs() < 1e-6);
        assert_eq!(peak(&square), 1.0);
    }

    #[test]
    fn peak_catches_what_rms_hides() {
        // One clipped sample in an otherwise ordinary block. The average is
        // unremarkable, so a meter that watched only the average would report
        // everything as fine — which is how clipping goes unnoticed until the
        // transcript is bad.
        let mut block = [0.01f32; 10_000];
        block[5_000] = 1.0;

        let (r, p) = (rms(&block), peak(&block));
        assert_ne!(
            verdict(r, r),
            Verdict::Clipping,
            "the average alone does not reveal it"
        );
        assert_eq!(verdict(r, p), Verdict::Clipping, "the peak does");
    }

    #[test]
    fn negative_samples_count_the_same_as_positive() {
        assert_eq!(peak(&[-0.8, 0.2]), 0.8);
    }

    #[test]
    fn half_amplitude_is_six_decibels_down() {
        assert!((dbfs(0.5) + 6.02).abs() < 0.05);
    }

    #[test]
    fn zero_has_a_finite_reading() {
        // −inf would poison every meter calculation downstream.
        assert_eq!(dbfs(0.0), -100.0);
        assert!(dbfs(0.0).is_finite());
    }

    #[test]
    fn an_empty_room_reads_as_silent_not_healthy() {
        // The threshold is not zero, because a live input never is. Room tone
        // sits around −60 dBFS, and calling that healthy would defeat the test.
        let room_tone = 0.0005; // about −66 dBFS
        assert_eq!(verdict(room_tone, room_tone), Verdict::Silent);
    }

    #[test]
    fn speech_at_a_normal_level_reads_healthy() {
        let speech = 0.05; // about −26 dBFS
        assert_eq!(verdict(speech, 0.2), Verdict::Healthy);
    }

    #[test]
    fn a_faint_signal_is_reported_as_quiet_not_absent() {
        // The distinction matters: quiet means the routing is right and the
        // gain is low; silent means the routing is wrong. They need different
        // fixes.
        let faint = 0.005; // about −46 dBFS
        assert_eq!(verdict(faint, 0.01), Verdict::Quiet);
    }

    #[test]
    fn the_meter_holds_a_peak_between_reads() {
        // A UI polling a few times a second would otherwise miss exactly the
        // transients worth seeing.
        let meter = LevelMeter::new();
        meter.observe(&[1.0, 0.0, 0.0, 0.0]);
        meter.observe(&[0.001; 4]);
        assert_eq!(meter.peak(), 1.0, "the peak was not held");
        assert!(meter.rms() < 0.01, "rms should follow the latest block");
    }

    #[test]
    fn resetting_clears_a_held_peak() {
        let meter = LevelMeter::new();
        meter.observe(&[1.0; 4]);
        meter.reset();
        assert_eq!(meter.peak(), 0.0);
        assert_eq!(meter.rms(), 0.0);
    }

    #[test]
    fn a_fresh_meter_reads_silent() {
        let meter = LevelMeter::new();
        assert_eq!(verdict(meter.rms(), meter.peak()), Verdict::Silent);
    }

    #[test]
    fn verdicts_have_stable_names_for_the_ui() {
        assert_eq!(Verdict::Silent.as_str(), "silent");
        assert_eq!(Verdict::Clipping.as_str(), "clipping");
    }
}
