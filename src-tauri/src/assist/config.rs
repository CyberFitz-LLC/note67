//! What live assistance is configured to do.
//!
//! Off unless deliberately turned on, and off again if the configuration is
//! half-written. The same rule the transcription backend follows, for a
//! stronger reason: this one sends the meeting to a model continuously while
//! people are still talking, and that is not something to start by accident.

pub const ENABLED_KEY: &str = "assist_enabled";
pub const HINDSIGHT_URL_KEY: &str = "assist_hindsight_url";
pub const HINDSIGHT_BANK_KEY: &str = "assist_hindsight_bank";

/// How assistance will actually run, once the settings are read.
#[derive(Debug, Clone, PartialEq)]
pub enum Assist {
    /// Not running. Nothing is sent anywhere.
    Off,
    /// Running, with an optional memory store behind it.
    ///
    /// Memory is optional on purpose: suggestions grounded only in the
    /// conversation are weaker but still useful, and refusing to run without a
    /// memory store would make a working feature depend on an unrelated
    /// service being up.
    On { memory: Option<MemorySource> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemorySource {
    pub base_url: String,
    pub bank: String,
}

/// Read the settings into a decision.
pub fn resolve(
    enabled: &str,
    hindsight_url: Option<&str>,
    hindsight_bank: Option<&str>,
) -> Assist {
    // Anything but an explicit yes is off. A half-written or hand-edited
    // setting must not start streaming a meeting to a model.
    if !matches!(enabled.trim(), "true" | "1" | "on") {
        return Assist::Off;
    }

    let url = hindsight_url.map(str::trim).unwrap_or_default();
    let bank = hindsight_bank.map(str::trim).unwrap_or_default();

    // Both or neither. A URL without a bank has nothing to ask, and a bank
    // without a URL has nowhere to ask it — either alone is a setting someone
    // started and did not finish.
    let memory = if !url.is_empty()
        && !bank.is_empty()
        && (url.starts_with("http://") || url.starts_with("https://"))
    {
        Some(MemorySource {
            base_url: url.to_string(),
            bank: bank.to_string(),
        })
    } else {
        None
    };

    Assist::On { memory }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistance_is_off_unless_it_was_turned_on() {
        // The default matters more here than for most settings: this one sends
        // a live meeting to a model.
        assert_eq!(resolve("", None, None), Assist::Off);
        assert_eq!(resolve("false", None, None), Assist::Off);
        assert_eq!(resolve("yes", None, None), Assist::Off);
        assert_eq!(resolve("TRUE", None, None), Assist::Off);
    }

    #[test]
    fn it_runs_when_switched_on() {
        assert_eq!(resolve("true", None, None), Assist::On { memory: None });
        assert_eq!(resolve("1", None, None), Assist::On { memory: None });
    }

    #[test]
    fn a_configured_bank_is_used() {
        assert_eq!(
            resolve("true", Some("https://hindsight.jtpa.net"), Some("john")),
            Assist::On {
                memory: Some(MemorySource {
                    base_url: "https://hindsight.jtpa.net".into(),
                    bank: "john".into(),
                })
            }
        );
    }

    #[test]
    fn half_a_memory_setting_is_no_memory_setting() {
        // A URL with no bank has nothing to ask; a bank with no URL has nowhere
        // to ask it. Running without memory is right — refusing to run at all
        // would make this feature depend on an unrelated service.
        assert_eq!(
            resolve("true", Some("https://hindsight.jtpa.net"), None),
            Assist::On { memory: None }
        );
        assert_eq!(
            resolve("true", None, Some("john")),
            Assist::On { memory: None }
        );
        assert_eq!(
            resolve("true", Some("hindsight.jtpa.net"), Some("john")),
            Assist::On { memory: None },
            "a URL with no scheme is not usable"
        );
    }

    #[test]
    fn whitespace_is_not_a_setting() {
        assert_eq!(
            resolve("true", Some("   "), Some("  ")),
            Assist::On { memory: None }
        );
    }
}
