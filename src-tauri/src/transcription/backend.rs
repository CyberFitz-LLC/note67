//! Which recogniser transcribes an upload.
//!
//! An enum rather than a trait object, for the same reason `ai::provider` is
//! one: `async fn` in traits is not `dyn`-compatible, the set of backends is
//! closed, and the alternatives buy nothing.
//!
//! **Live transcription is not routed through here.** It runs on three-second
//! chunks while a meeting is in progress; a remote round trip per chunk would
//! be slower than the meeting, and a network blip would drop words nobody can
//! get back. Uploads are a finished file with no deadline, which is exactly
//! where a slower, better recogniser belongs.

use serde::{Deserialize, Serialize};

/// Settings keys, so the strings live in one place.
pub const BACKEND_KEY: &str = "transcription_backend";
pub const BASE_URL_KEY: &str = "transcription_base_url";
pub const API_KEY_KEY: &str = "transcription_api_key";
pub const MAX_SPEAKERS_KEY: &str = "transcription_max_speakers";
pub const STREAM_URL_KEY: &str = "transcription_stream_url";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    /// Whisper, in this process. No audio leaves the machine.
    #[default]
    Local,
    /// A `note67-asr` service: transcription plus diarization, at the cost of
    /// sending the recording somewhere.
    Remote,
    /// A streaming recogniser, live over a websocket. Better than local Whisper
    /// and, unlike `Remote`, works while the meeting is happening — but audio
    /// leaves the machine continuously rather than as one file afterwards, and
    /// it does not diarize.
    Streaming,
}

impl BackendKind {
    /// Parse a persisted value.
    ///
    /// Anything unrecognised is Local. A hand-edited or partly-written setting
    /// must not silently start shipping recordings off the machine — the
    /// failure has to fall towards keeping audio at home.
    pub fn from_setting(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "remote" | "note67_asr" | "note67-asr" => BackendKind::Remote,
            "streaming" | "stream" => BackendKind::Streaming,
            _ => BackendKind::Local,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            BackendKind::Local => "local",
            BackendKind::Remote => "remote",
            BackendKind::Streaming => "streaming",
        }
    }
}

/// A resolved backend, with everything needed to run it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backend {
    Local,
    Remote {
        base_url: String,
        api_key: Option<String>,
        /// An upper bound on speakers, when the user knows it. The diarizer
        /// infers the count on its own; this only stops it inventing more.
        max_speakers: Option<u32>,
    },
    Streaming {
        /// The websocket endpoint. Two connections are opened against it, one
        /// per track, because mixing the microphone and system audio into a
        /// single stream would discard the You/Others distinction that is the
        /// app's only attribution without a diarizer.
        ws_url: String,
    },
}

/// Decide from settings.
///
/// Falls back to Local whenever the remote backend is chosen but not usable —
/// a URL that is blank or unparseable means the setting was never finished, and
/// failing the transcription outright would be a worse answer than the one that
/// has always worked.
pub fn resolve(
    kind: &str,
    base_url: Option<&str>,
    api_key: Option<&str>,
    max_speakers: Option<&str>,
    stream_url: Option<&str>,
) -> Backend {
    match BackendKind::from_setting(kind) {
        BackendKind::Local => Backend::Local,
        BackendKind::Streaming => {
            let url = stream_url.map(str::trim).unwrap_or_default();
            // Same rule as Remote, and for the same reason: a half-written
            // setting must not start streaming a live meeting off the machine.
            if url.is_empty() || !(url.starts_with("ws://") || url.starts_with("wss://")) {
                return Backend::Local;
            }
            Backend::Streaming {
                ws_url: url.trim_end_matches('/').to_string(),
            }
        }
        BackendKind::Remote => {
            let url = base_url.map(str::trim).unwrap_or_default();
            if url.is_empty() || !(url.starts_with("http://") || url.starts_with("https://")) {
                return Backend::Local;
            }
            Backend::Remote {
                base_url: url.trim_end_matches('/').to_string(),
                api_key: api_key
                    .map(str::trim)
                    .filter(|k| !k.is_empty())
                    .map(str::to_string),
                max_speakers: max_speakers.and_then(|m| m.trim().parse().ok()).filter(|m| *m > 0),
            }
        }
    }
}

/// Ask a streaming service whether it is up before relying on it.
///
/// Deliberately not part of `resolve`, which is pure and synchronous and whose
/// tests would otherwise need a network. The setting is validated here, at the
/// point someone saves it, the way a blank URL is caught today.
pub async fn stream_health(ws_url: &str) -> Result<(), String> {
    // The health endpoint is HTTP on the same host and port as the websocket.
    let http = ws_url
        .trim()
        .replacen("wss://", "https://", 1)
        .replacen("ws://", "http://", 1);
    let url = format!("{}/health", http.trim_end_matches('/'));

    let response = reqwest::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| format!("could not reach the recogniser: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("the recogniser returned {}", response.status()));
    }

    // Up is not the same as ready. A service whose model has not loaded will
    // accept a socket and transcribe nothing, which looks like a broken
    // microphone rather than a service still starting.
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("the recogniser's answer could not be read: {e}"))?;
    if body.get("model_loaded") == Some(&serde_json::Value::Bool(false)) {
        return Err("the recogniser is running but its model is not loaded".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_keeps_audio_on_the_machine() {
        assert_eq!(BackendKind::default(), BackendKind::Local);
    }

    #[test]
    fn an_unrecognised_setting_falls_back_to_local() {
        // The failure has to fall towards keeping audio at home. A hand-edited
        // or half-written setting must never start shipping recordings.
        for v in ["", "  ", "elsewhere", "REMOTE-ish", "true"] {
            assert_eq!(BackendKind::from_setting(v), BackendKind::Local, "{v:?}");
        }
    }

    #[test]
    fn remote_is_recognised_by_its_names() {
        for v in ["remote", "REMOTE", " note67-asr ", "note67_asr"] {
            assert_eq!(BackendKind::from_setting(v), BackendKind::Remote, "{v:?}");
        }
    }

    #[test]
    fn the_stored_form_round_trips() {
        for k in [BackendKind::Local, BackendKind::Remote] {
            assert_eq!(BackendKind::from_setting(k.as_str()), k);
        }
    }

    #[test]
    fn a_configured_remote_resolves() {
        let b = resolve(
            "remote",
            Some("http://192.168.32.223:8010/"),
            Some("secret"),
            Some("8"),
            None,
        );
        assert_eq!(
            b,
            Backend::Remote {
                // Trailing slash removed once here rather than at every call
                // site that builds a path onto it.
                base_url: "http://192.168.32.223:8010".into(),
                api_key: Some("secret".into()),
                max_speakers: Some(8),
            }
        );
    }

    #[test]
    fn remote_without_a_url_falls_back_rather_than_failing() {
        // Choosing the backend and not finishing the setting is an ordinary
        // half-done state, and the transcription that has always worked is a
        // better answer than an error.
        assert_eq!(resolve("remote", None, None, None, None), Backend::Local);
        assert_eq!(resolve("remote", Some("   "), None, None, None), Backend::Local);
    }

    #[test]
    fn a_url_without_a_scheme_is_refused() {
        // Otherwise the first sign of trouble is a confusing request error
        // rather than a setting that was never valid.
        assert_eq!(
            resolve("remote", Some("192.168.32.223:8010"), None, None, None),
            Backend::Local
        );
    }

    #[test]
    fn an_absent_api_key_stays_absent() {
        let b = resolve("remote", Some("http://x:8010"), Some("  "), None, None);
        assert!(matches!(b, Backend::Remote { api_key: None, .. }));
    }

    #[test]
    fn streaming_is_recognised_and_resolves() {
        assert_eq!(BackendKind::from_setting("streaming"), BackendKind::Streaming);
        assert_eq!(
            resolve("streaming", None, None, None, Some("ws://192.168.32.223:8080/")),
            Backend::Streaming {
                ws_url: "ws://192.168.32.223:8080".into()
            }
        );
    }

    #[test]
    fn streaming_without_a_usable_url_falls_back_to_local() {
        // A live meeting must not start streaming off the machine because a
        // setting was half written. Same rule as Remote, higher stakes: this
        // one sends audio continuously while recording.
        for url in [None, Some("  "), Some("192.168.32.223:8080"), Some("http://x:8080")] {
            assert_eq!(
                resolve("streaming", None, None, None, url),
                Backend::Local,
                "{url:?}"
            );
        }
    }

    #[test]
    fn a_nonsense_speaker_cap_is_ignored_not_obeyed() {
        // The diarizer infers the count. A zero or unparseable cap should
        // leave it to do that rather than constrain it to nothing.
        for v in ["0", "-3", "lots", ""] {
            let b = resolve("remote", Some("http://x:8010"), None, Some(v), None);
            assert!(
                matches!(b, Backend::Remote { max_speakers: None, .. }),
                "{v:?}"
            );
        }
    }
}
