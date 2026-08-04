//! Input-device enumeration and selection.
//!
//! Recording used to always open `default_input_device()`. This module lets the
//! user pin a specific microphone instead, while keeping the old behaviour as
//! the fallback whenever the pinned device is gone (unplugged, renamed, or on a
//! different machine that shares the same settings database).

use std::collections::HashSet;

use cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};

use crate::audio::AudioError;

/// An input device the user can pick for recording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioInputDevice {
    pub name: String,
    /// Whether this is the host's current default input device.
    pub is_default: bool,
}

/// Which device a recording should open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputSelection {
    /// No preference saved — follow the system default, including when the user
    /// changes it in the OS while the app is running.
    Default,
    /// Open the named device.
    Named(String),
    /// A device was pinned but is not present right now. Recording still has to
    /// happen, so this falls back to the default and reports what was missed.
    MissingFallback { requested: String },
}

/// Decide which device to open, given the devices present and the saved
/// preference.
///
/// Split out from the cpal calls so the fallback rules can be tested without
/// audio hardware, which CI does not have.
pub fn resolve_input_selection(available: &[String], preferred: Option<&str>) -> InputSelection {
    match preferred {
        // Both "never set" and "explicitly cleared" mean follow the system.
        None => InputSelection::Default,
        Some(name) if name.trim().is_empty() => InputSelection::Default,
        // Exact match only: cpal addresses devices by their exact name, so a
        // fuzzy match here would open a device the user did not pick.
        Some(name) if available.iter().any(|d| d == name) => InputSelection::Named(name.to_string()),
        Some(name) => InputSelection::MissingFallback {
            requested: name.to_string(),
        },
    }
}

/// Collect the names of every input device the host can see.
fn input_device_names(host: &cpal::Host) -> Result<Vec<String>, AudioError> {
    Ok(host.input_devices()?.filter_map(|d| d.name().ok()).collect())
}

/// List the input devices available for recording.
///
/// Devices whose names collide are deduplicated: cpal can only be asked for a
/// device *by name*, so offering the user a second "USB Audio" we could never
/// actually open would be a lie.
pub fn list_input_devices() -> Result<Vec<AudioInputDevice>, AudioError> {
    let host = cpal::default_host();
    let default_name = host.default_input_device().and_then(|d| d.name().ok());

    let mut seen = HashSet::new();
    let mut devices = Vec::new();

    for name in input_device_names(&host)? {
        if !seen.insert(name.clone()) {
            continue;
        }
        let is_default = default_name.as_deref() == Some(name.as_str());
        devices.push(AudioInputDevice { name, is_default });
    }

    Ok(devices)
}

/// Open the input device the user picked, falling back to the system default.
pub fn open_input_device(preferred: Option<&str>) -> Result<cpal::Device, AudioError> {
    let host = cpal::default_host();
    let available = input_device_names(&host)?;

    match resolve_input_selection(&available, preferred) {
        InputSelection::Named(name) => host
            .input_devices()?
            .find(|d| d.name().map(|n| n == name).unwrap_or(false))
            // Only reachable if the device disappeared between the two
            // enumerations above.
            .ok_or(AudioError::NoInputDevice),
        InputSelection::MissingFallback { requested } => {
            eprintln!(
                "Saved input device {:?} is not available; recording with the system default instead",
                requested
            );
            host.default_input_device().ok_or(AudioError::NoInputDevice)
        }
        InputSelection::Default => host.default_input_device().ok_or(AudioError::NoInputDevice),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn available() -> Vec<String> {
        vec![
            "MacBook Pro Microphone".to_string(),
            "Blue Yeti".to_string(),
        ]
    }

    #[test]
    fn no_preference_follows_the_system_default() {
        assert_eq!(
            resolve_input_selection(&available(), None),
            InputSelection::Default
        );
    }

    #[test]
    fn a_cleared_preference_follows_the_system_default() {
        // The settings store is string-typed, so "cleared" arrives as an empty
        // string rather than as None.
        assert_eq!(
            resolve_input_selection(&available(), Some("")),
            InputSelection::Default
        );
        assert_eq!(
            resolve_input_selection(&available(), Some("   ")),
            InputSelection::Default
        );
    }

    #[test]
    fn a_present_device_is_selected() {
        assert_eq!(
            resolve_input_selection(&available(), Some("Blue Yeti")),
            InputSelection::Named("Blue Yeti".to_string())
        );
    }

    #[test]
    fn an_absent_device_falls_back_rather_than_failing() {
        // Unplugging the pinned mic must not stop the user recording.
        assert_eq!(
            resolve_input_selection(&available(), Some("Blue Yeti")),
            InputSelection::Named("Blue Yeti".to_string())
        );
        assert_eq!(
            resolve_input_selection(&["MacBook Pro Microphone".to_string()], Some("Blue Yeti")),
            InputSelection::MissingFallback {
                requested: "Blue Yeti".to_string()
            }
        );
    }

    #[test]
    fn matching_is_exact() {
        // cpal opens devices by exact name; a near-match would silently record
        // from the wrong microphone.
        assert!(matches!(
            resolve_input_selection(&available(), Some("blue yeti")),
            InputSelection::MissingFallback { .. }
        ));
        assert!(matches!(
            resolve_input_selection(&available(), Some("Yeti")),
            InputSelection::MissingFallback { .. }
        ));
        assert!(matches!(
            resolve_input_selection(&available(), Some("Blue Yeti 2")),
            InputSelection::MissingFallback { .. }
        ));
    }

    #[test]
    fn an_empty_host_still_falls_back_to_the_default() {
        // open_input_device turns this into NoInputDevice; the resolver's job is
        // only to say "not the pinned one".
        assert_eq!(
            resolve_input_selection(&[], Some("Blue Yeti")),
            InputSelection::MissingFallback {
                requested: "Blue Yeti".to_string()
            }
        );
        assert_eq!(resolve_input_selection(&[], None), InputSelection::Default);
    }
}
