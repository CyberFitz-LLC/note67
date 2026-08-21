//! Checking the audio routing before a meeting rather than during one.
//!
//! The device is bound when a stream opens, so changing it mid-recording does
//! nothing until the next one. That is worth fixing separately; it is not worth
//! *relying* on, because the thing a user actually needs is to know the routing
//! is right before the meeting starts.
//!
//! The test drives the same functions a recording drives — `start_recording`
//! for the microphone and the platform's `SystemAudioCapture` for the other
//! track — writing to a temporary directory that is deleted afterwards. A test
//! that opened its own streams could pass while recording failed, which is
//! worse than no test.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::audio::levels::{dbfs, verdict};
use crate::audio::system_audio::system_level;
use crate::commands::audio::AudioState;

/// Where a test in progress is writing, so it can be cleaned up.
#[derive(Default)]
pub struct DeviceTestState {
    pub temp_dir: Mutex<Option<PathBuf>>,
}

/// One track's reading.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackLevel {
    /// Decibels below full scale. Meters are read in dB because hearing is
    /// logarithmic — a linear bar spends most of its length on differences
    /// nobody can hear.
    pub rms_dbfs: f32,
    pub peak_dbfs: f32,
    /// `silent` | `quiet` | `healthy` | `clipping`.
    pub verdict: String,
}

fn track(rms: f32, peak: f32) -> TrackLevel {
    TrackLevel {
        rms_dbfs: dbfs(rms),
        peak_dbfs: dbfs(peak),
        verdict: verdict(rms, peak).as_str().to_string(),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceTestLevels {
    pub microphone: TrackLevel,
    pub system: TrackLevel,
    /// The devices each track actually opened.
    ///
    /// Reported rather than assumed. A pinned device that has gone away falls
    /// back to the default silently, so the picker's value does not say what a
    /// meter is reading — and a meter you cannot attribute to a device is not
    /// evidence of anything.
    pub microphone_device: Option<String>,
    pub system_device: Option<String>,
    /// False when the system track could not be started at all — no capture
    /// support, or no permission. Distinct from a silent track, which means the
    /// capture works and is hearing nothing.
    pub system_available: bool,
}

/// Start both captures, writing to a temporary directory.
#[tauri::command]
pub fn start_device_test(
    app: AppHandle,
    audio: State<AudioState>,
    test: State<DeviceTestState>,
) -> Result<bool, String> {
    if audio.recording.is_recording.load(std::sync::atomic::Ordering::SeqCst) {
        // Both would fight for the same device and the same writer. Refusing is
        // better than a test that reports on a recording already in progress.
        return Err("Stop the recording before testing devices.".into());
    }

    stop_device_test(audio.clone(), test.clone()).ok();

    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("could not locate the app data directory: {e}"))?
        .join("device-test");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    system_level().reset();
    audio.recording.audio_level.store(0f32.to_bits(), std::sync::atomic::Ordering::SeqCst);

    crate::audio::start_recording(audio.recording.clone(), dir.join("mic.wav"))
        .map_err(|e| e.to_string())?;

    // The system track is allowed to fail without failing the test: on a
    // machine with no loopback support the microphone half is still worth
    // checking, and saying so is more useful than refusing to run.
    let system_started = match audio.system_capture.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(capture) => capture.start(dir.join("system.wav")).is_ok(),
            None => false,
        },
        Err(_) => false,
    };

    *test.temp_dir.lock().map_err(|e| e.to_string())? = Some(dir);
    Ok(system_started)
}

/// Stop both captures and remove what they wrote.
///
/// The recordings are deleted: a device test is not a meeting, and leaving
/// audio of one behind would be a surprise.
#[tauri::command]
pub fn stop_device_test(audio: State<AudioState>, test: State<DeviceTestState>) -> Result<(), String> {
    let _ = crate::audio::stop_recording(&audio.recording);
    if let Ok(guard) = audio.system_capture.lock()
        && let Some(capture) = guard.as_ref()
    {
        let _ = capture.stop();
    }

    if let Ok(mut dir) = test.temp_dir.lock()
        && let Some(path) = dir.take()
    {
        let _ = std::fs::remove_dir_all(path);
    }
    Ok(())
}

#[tauri::command]
pub fn get_device_test_levels(audio: State<AudioState>) -> DeviceTestLevels {
    let mic_rms = f32::from_bits(
        audio
            .recording
            .audio_level
            .load(std::sync::atomic::Ordering::SeqCst),
    );

    let (sys_rms, sys_peak, available) = match audio.system_capture.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(capture) => {
                let (r, p) = capture.current_level();
                (r, p, capture.is_capturing())
            }
            None => (0.0, 0.0, false),
        },
        Err(_) => (0.0, 0.0, false),
    };

    DeviceTestLevels {
        // The recorder tracks only an average for the microphone, so peak is
        // reported as the same value rather than invented. Clipping on the mic
        // therefore reads as "healthy" until the recorder carries a real peak —
        // noted rather than papered over.
        microphone: track(mic_rms, mic_rms),
        system: track(sys_rms, sys_peak),
        microphone_device: audio
            .recording
            .opened_input_device
            .lock()
            .ok()
            .and_then(|d| d.clone()),
        system_device: crate::audio::system_audio::system_device(),
        system_available: available,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_silent_track_reports_silence_not_an_error() {
        let t = track(0.0, 0.0);
        assert_eq!(t.verdict, "silent");
        assert!(t.rms_dbfs.is_finite(), "a meter must never read -inf");
    }

    #[test]
    fn a_speaking_track_reads_healthy() {
        assert_eq!(track(0.05, 0.2).verdict, "healthy");
    }

    #[test]
    fn a_clipping_peak_is_reported_even_when_the_average_is_ordinary() {
        // The case an average-only meter misses, and the reason the system
        // track carries a held peak.
        assert_eq!(track(0.05, 1.0).verdict, "clipping");
    }

    #[test]
    fn levels_serialize_under_the_names_the_ui_reads() {
        let levels = DeviceTestLevels {
            microphone: track(0.05, 0.05),
            system: track(0.0, 0.0),
            microphone_device: Some("Blue Yeti".into()),
            system_device: Some("VoiceMeeter Out B1 — recording device".into()),
            system_available: true,
        };
        let v = serde_json::to_value(&levels).unwrap();
        assert_eq!(v["microphone"]["verdict"], "healthy");
        assert_eq!(v["system"]["verdict"], "silent");
        assert_eq!(v["systemAvailable"], true);
        assert!(v["microphone"]["rmsDbfs"].is_number());
        // Which device each meter is reading, so a moving bar can be
        // attributed rather than guessed at.
        assert_eq!(v["microphoneDevice"], "Blue Yeti");
        assert_eq!(v["systemDevice"], "VoiceMeeter Out B1 — recording device");
    }

    #[test]
    fn an_unavailable_system_track_is_distinct_from_a_silent_one() {
        // Different problems: unavailable means the capture never started,
        // silent means it started and is hearing nothing. The first is a
        // platform or permission issue, the second is a routing one.
        let unavailable = DeviceTestLevels {
            microphone: track(0.05, 0.05),
            system: track(0.0, 0.0),
            microphone_device: Some("Blue Yeti".into()),
            system_device: None,
            system_available: false,
        };
        assert_eq!(unavailable.system.verdict, "silent");
        assert!(!unavailable.system_available);
    }
}
