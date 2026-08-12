//! Platform abstraction for system audio capture.
//!
//! System audio capture allows recording audio output from the system,
//! which is used to capture meeting participants' voices.

use std::path::PathBuf;
use std::sync::Arc;

use crate::audio::levels::LevelMeter;
use crate::audio::AudioError;

/// What the system-audio track is currently hearing.
///
/// Global, and platform-neutral, because the capture implementations are
/// per-platform statics already and threading a handle through each of them
/// would buy nothing. Until now nothing measured this track at all — the only
/// meter in the app was the microphone's, which is why changing the system
/// device appeared to do nothing.
static SYSTEM_LEVEL: std::sync::OnceLock<LevelMeter> = std::sync::OnceLock::new();

pub fn system_level() -> &'static LevelMeter {
    SYSTEM_LEVEL.get_or_init(LevelMeter::new)
}

/// Result type for system audio operations
pub type SystemAudioResult<T> = Result<T, AudioError>;

/// Platform-agnostic interface for system audio capture
pub trait SystemAudioCapture: Send + Sync {
    /// Check if system audio capture is supported on this platform
    fn is_supported() -> bool
    where
        Self: Sized;

    /// Check if the app has permission to capture system audio
    fn has_permission(&self) -> SystemAudioResult<bool>;

    /// Request permission to capture system audio
    /// Returns true if permission was granted
    fn request_permission(&self) -> SystemAudioResult<bool>;

    /// Start capturing system audio to the specified file
    fn start(&self, output_path: PathBuf) -> SystemAudioResult<()>;

    /// Stop capturing system audio
    /// Returns the path to the recorded file
    fn stop(&self) -> SystemAudioResult<Option<PathBuf>>;

    /// Check if currently capturing
    fn is_capturing(&self) -> bool;

    /// Pin a playback device to capture from, by name. `None` follows the
    /// system default.
    ///
    /// Defaults to accepting and ignoring the choice. macOS captures the whole
    /// system mix through ScreenCaptureKit, where there is no output device to
    /// choose between — the equivalent knob there is which *applications* to
    /// include, which is a different feature.
    fn set_preferred_device(&self, _name: Option<String>) -> SystemAudioResult<()> {
        Ok(())
    }

    /// What this track is hearing right now, as (rms, held peak).
    ///
    /// Reads the shared meter rather than being implemented per platform: every
    /// capture path already pushes its samples through one place.
    fn current_level(&self) -> (f32, f32) {
        (system_level().rms(), system_level().peak())
    }

    /// The pinned playback device, if this platform supports choosing one.
    fn get_preferred_device(&self) -> Option<String> {
        None
    }
}

/// List the playback devices whose audio can be captured.
///
/// Empty on platforms where the choice does not exist; pair it with
/// [`is_output_device_selectable`] to tell "none found" apart from
/// "not applicable here".
pub fn list_output_devices() -> SystemAudioResult<Vec<crate::audio::AudioDevice>> {
    #[cfg(target_os = "windows")]
    {
        super::windows::list_render_devices()
    }

    #[cfg(not(target_os = "windows"))]
    {
        Ok(Vec::new())
    }
}

/// Whether this platform lets the user choose which playback device is captured.
pub fn is_output_device_selectable() -> bool {
    cfg!(target_os = "windows")
}

/// Get the system audio capture implementation for the current platform
#[cfg(target_os = "macos")]
pub fn create_system_audio_capture() -> SystemAudioResult<Arc<dyn SystemAudioCapture>> {
    use super::macos::MacOSSystemAudioCapture;
    Ok(Arc::new(MacOSSystemAudioCapture::new()))
}

#[cfg(target_os = "windows")]
pub fn create_system_audio_capture() -> SystemAudioResult<Arc<dyn SystemAudioCapture>> {
    use super::windows::WindowsSystemAudioCapture;
    Ok(Arc::new(WindowsSystemAudioCapture::new()?))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn create_system_audio_capture() -> SystemAudioResult<Arc<dyn SystemAudioCapture>> {
    Err(AudioError::UnsupportedPlatform)
}

/// Check if system audio capture is available on the current platform
pub fn is_system_audio_available() -> bool {
    #[cfg(target_os = "macos")]
    {
        super::macos::MacOSSystemAudioCapture::is_supported()
    }
    #[cfg(target_os = "windows")]
    {
        super::windows::WindowsSystemAudioCapture::is_supported()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        false
    }
}
