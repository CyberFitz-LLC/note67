//! Audio-device enumeration and selection.
//!
//! Recording used to always open `default_input_device()`. This module lets the
//! user pin a specific microphone instead, while keeping the old behaviour as
//! the fallback whenever the pinned device is gone (unplugged, renamed, or on a
//! different machine that shares the same settings database).
//!
//! `resolve_device_selection` is deliberately direction-agnostic: the Windows
//! loopback capture picks a *playback* device by exactly the same rules, and
//! two copies of those rules would drift.

use std::collections::HashSet;

use cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};

use crate::audio::AudioError;

/// A device the user can pick — a microphone to record from, or (on Windows) a
/// playback device to capture system audio from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    /// Stable across restarts and unique per endpoint. What a preference
    /// stores, because a name is not necessarily either.
    ///
    /// Empty where the platform exposes no id — cpal's input devices — in which
    /// case the name is all there is and selection stays name-based.
    #[serde(default)]
    pub id: String,
    pub name: String,
    /// Whether this is the operating system's current default for its direction.
    pub is_default: bool,
}

/// Which device a recording should open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceSelection {
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
pub fn resolve_device_selection(available: &[String], preferred: Option<&str>) -> DeviceSelection {
    match preferred {
        // Both "never set" and "explicitly cleared" mean follow the system.
        None => DeviceSelection::Default,
        Some(name) if name.trim().is_empty() => DeviceSelection::Default,
        // Exact match only: cpal addresses devices by their exact name, so a
        // fuzzy match here would open a device the user did not pick.
        Some(name) if available.iter().any(|d| d == name) => DeviceSelection::Named(name.to_string()),
        Some(name) => DeviceSelection::MissingFallback {
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
pub fn list_input_devices() -> Result<Vec<AudioDevice>, AudioError> {
    let host = cpal::default_host();
    let default_name = host.default_input_device().and_then(|d| d.name().ok());

    let mut seen = HashSet::new();
    let mut devices = Vec::new();

    for name in input_device_names(&host)? {
        if !seen.insert(name.clone()) {
            continue;
        }
        let is_default = default_name.as_deref() == Some(name.as_str());
        // cpal exposes no endpoint id, so microphones stay name-selected. They
        // rarely duplicate the way playback endpoints do, and inventing an id
        // that is really the name would hide that difference rather than fix it.
        devices.push(AudioDevice {
            id: String::new(),
            name,
            is_default,
        });
    }

    Ok(devices)
}

/// Open the input device the user picked, falling back to the system default.
pub fn open_input_device(preferred: Option<&str>) -> Result<cpal::Device, AudioError> {
    let host = cpal::default_host();
    let available = input_device_names(&host)?;

    match resolve_device_selection(&available, preferred) {
        DeviceSelection::Named(name) => host
            .input_devices()?
            .find(|d| d.name().map(|n| n == name).unwrap_or(false))
            // Only reachable if the device disappeared between the two
            // enumerations above.
            .ok_or(AudioError::NoInputDevice),
        DeviceSelection::MissingFallback { requested } => {
            eprintln!(
                "Saved input device {:?} is not available; recording with the system default instead",
                requested
            );
            host.default_input_device().ok_or(AudioError::NoInputDevice)
        }
        DeviceSelection::Default => host.default_input_device().ok_or(AudioError::NoInputDevice),
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
            resolve_device_selection(&available(), None),
            DeviceSelection::Default
        );
    }

    #[test]
    fn a_cleared_preference_follows_the_system_default() {
        // The settings store is string-typed, so "cleared" arrives as an empty
        // string rather than as None.
        assert_eq!(
            resolve_device_selection(&available(), Some("")),
            DeviceSelection::Default
        );
        assert_eq!(
            resolve_device_selection(&available(), Some("   ")),
            DeviceSelection::Default
        );
    }

    #[test]
    fn a_present_device_is_selected() {
        assert_eq!(
            resolve_device_selection(&available(), Some("Blue Yeti")),
            DeviceSelection::Named("Blue Yeti".to_string())
        );
    }

    #[test]
    fn an_absent_device_falls_back_rather_than_failing() {
        // Unplugging the pinned mic must not stop the user recording.
        assert_eq!(
            resolve_device_selection(&available(), Some("Blue Yeti")),
            DeviceSelection::Named("Blue Yeti".to_string())
        );
        assert_eq!(
            resolve_device_selection(&["MacBook Pro Microphone".to_string()], Some("Blue Yeti")),
            DeviceSelection::MissingFallback {
                requested: "Blue Yeti".to_string()
            }
        );
    }

    #[test]
    fn matching_is_exact() {
        // cpal opens devices by exact name; a near-match would silently record
        // from the wrong microphone.
        assert!(matches!(
            resolve_device_selection(&available(), Some("blue yeti")),
            DeviceSelection::MissingFallback { .. }
        ));
        assert!(matches!(
            resolve_device_selection(&available(), Some("Yeti")),
            DeviceSelection::MissingFallback { .. }
        ));
        assert!(matches!(
            resolve_device_selection(&available(), Some("Blue Yeti 2")),
            DeviceSelection::MissingFallback { .. }
        ));
    }

    #[test]
    fn an_empty_host_still_falls_back_to_the_default() {
        // open_input_device turns this into NoInputDevice; the resolver's job is
        // only to say "not the pinned one".
        assert_eq!(
            resolve_device_selection(&[], Some("Blue Yeti")),
            DeviceSelection::MissingFallback {
                requested: "Blue Yeti".to_string()
            }
        );
        assert_eq!(resolve_device_selection(&[], None), DeviceSelection::Default);
    }
}

/// A device as the platform enumerated it: a stable id, and a name that may
/// not be unique.
///
/// Windows can present several endpoints under one friendly name — VoiceMeeter
/// installs seven "Speakers (VB-Audio Voicemeeter VAIO)" — and only one of them
/// is the one an application is actually playing into. Selecting by name picks
/// whichever came first, which is why choosing the right-looking device could
/// capture silence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceEntry {
    pub id: String,
    pub name: String,
}

/// Give every device a label the user can tell apart.
///
/// Names that occur once are left alone. A repeated name is numbered in
/// enumeration order, which is the order the operating system lists them and
/// therefore the order anything else showing these devices will use.
pub fn disambiguate(entries: &[DeviceEntry]) -> Vec<String> {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for e in entries {
        *counts.entry(e.name.as_str()).or_default() += 1;
    }

    let mut seen: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    entries
        .iter()
        .map(|e| {
            if counts.get(e.name.as_str()).copied().unwrap_or(0) <= 1 {
                return e.name.clone();
            }
            let n = seen.entry(e.name.as_str()).or_default();
            *n += 1;
            format!("{} ({})", e.name, n)
        })
        .collect()
}

/// Find the device a saved preference refers to.
///
/// Matches on id first. Falling back to the name keeps preferences saved before
/// ids existed working — but it is a fallback precisely because it is the
/// ambiguous one, and it takes the first match just as the old code did.
pub fn find_device<'a>(entries: &'a [DeviceEntry], preferred: &str) -> Option<&'a DeviceEntry> {
    let wanted = preferred.trim();
    if wanted.is_empty() {
        return None;
    }
    entries
        .iter()
        .find(|e| e.id == wanted)
        .or_else(|| entries.iter().find(|e| e.name == wanted))
}

#[cfg(test)]
mod entry_tests {
    use super::*;

    fn entry(id: &str, name: &str) -> DeviceEntry {
        DeviceEntry {
            id: id.into(),
            name: name.into(),
        }
    }

    fn voicemeeter() -> Vec<DeviceEntry> {
        // What Windows actually presents with VoiceMeeter Banana installed.
        (1..=7)
            .map(|i| entry(&format!("{{0.0.0.id}}\\vaio{i}"), "Speakers (VB-Audio Voicemeeter VAIO)"))
            .collect()
    }

    #[test]
    fn a_unique_name_is_left_alone() {
        let entries = vec![entry("a", "Headphones"), entry("b", "Monitor")];
        assert_eq!(disambiguate(&entries), vec!["Headphones", "Monitor"]);
    }

    #[test]
    fn repeated_names_are_numbered_in_enumeration_order() {
        // The order matters: it is the order Windows lists them, so "the sixth
        // one" means the same thing here as in any other application's picker.
        let labels = disambiguate(&voicemeeter());
        assert_eq!(labels.len(), 7);
        assert_eq!(labels[0], "Speakers (VB-Audio Voicemeeter VAIO) (1)");
        assert_eq!(labels[5], "Speakers (VB-Audio Voicemeeter VAIO) (6)");
    }

    #[test]
    fn every_duplicate_survives() {
        // The bug this replaces: seven endpoints collapsed to one entry, and
        // the six hidden ones included the only one that was carrying audio.
        assert_eq!(disambiguate(&voicemeeter()).len(), 7);
    }

    #[test]
    fn only_the_repeated_names_are_numbered() {
        let mut entries = voicemeeter();
        entries.push(entry("hp", "Headphones"));
        let labels = disambiguate(&entries);
        assert_eq!(labels[7], "Headphones", "a unique name gained a number");
    }

    #[test]
    fn a_device_is_found_by_id() {
        let entries = voicemeeter();
        let found = find_device(&entries, "{0.0.0.id}\\vaio6").unwrap();
        assert_eq!(found.id, "{0.0.0.id}\\vaio6");
    }

    #[test]
    fn an_id_beats_a_name_when_both_could_match() {
        // The whole point. With seven identical names, the id is the only thing
        // that says which endpoint was chosen.
        let entries = voicemeeter();
        let found = find_device(&entries, "{0.0.0.id}\\vaio6").unwrap();
        assert_eq!(found.id, "{0.0.0.id}\\vaio6");
        assert_ne!(found.id, entries[0].id);
    }

    #[test]
    fn a_name_still_resolves_for_preferences_saved_before_ids() {
        // Ambiguously, and to the first match — exactly as the old code did.
        // Upgrading someone should not silently drop their choice.
        let entries = voicemeeter();
        let found = find_device(&entries, "Speakers (VB-Audio Voicemeeter VAIO)").unwrap();
        assert_eq!(found.id, entries[0].id);
    }

    #[test]
    fn an_unknown_preference_finds_nothing() {
        assert!(find_device(&voicemeeter(), "Some departed device").is_none());
    }

    #[test]
    fn an_empty_preference_finds_nothing() {
        // Distinct from "not found": an empty preference means follow the
        // default, and matching it to a device would pin one at random.
        assert!(find_device(&voicemeeter(), "  ").is_none());
    }
}
