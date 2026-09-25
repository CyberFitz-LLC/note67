//! Transcription by NeMo-Speech.cpp, on this machine.
//!
//! The third way to transcribe, and the only one that both diarizes and keeps
//! the audio on the device. Local Whisper hears one voice per track and cannot
//! tell speakers apart at all; the remote backends can, at the cost of sending
//! the recording somewhere.
//!
//! A child process rather than a library: NVIDIA ship a CLI with a documented
//! JSON contract, and its `serve` mode — which would have fitted the existing
//! OpenAI backend — does not return speaker tags in v0.1.0. Measured at ~8.8x
//! real time on a mobile Quadro.
//!
//! The runtime is **not** bundled. It is fetched once, like a Whisper model,
//! so the installer stays small across frequent releases.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::Deserialize;
use thiserror::Error;

use super::transcriber::{TranscriptionResult, TranscriptionSegment};

/// Settings keys.
pub const NEMO_PATH_KEY: &str = "transcription_nemo_path";
pub const NEMO_ASR_MODEL_KEY: &str = "transcription_nemo_asr_model";
pub const NEMO_DIAR_MODEL_KEY: &str = "transcription_nemo_diar_model";

/// What to ask for when nothing is configured.
///
/// Indexed names rather than paths: the CLI resolves them against its own
/// cache and fetches what is missing, with a SHA-256 check it performs itself.
pub const DEFAULT_ASR_MODEL: &str = "nemotron-3.5";
pub const DEFAULT_DIAR_MODEL: &str = "sortformer";

/// How long one transcription may take.
///
/// Measured at roughly nine times real time, so an hour of audio lands in
/// about seven minutes. Generous enough to survive a far slower machine — a
/// CPU-only build is an order of magnitude down — without hanging forever on a
/// process that has stopped making progress.
pub const TIMEOUT_SECS: u64 = 3 * 60 * 60;

#[derive(Debug, Error)]
pub enum NemoError {
    #[error("the local recogniser was not found at {0}")]
    NotFound(String),
    #[error("the local recogniser could not be started: {0}")]
    Spawn(String),
    #[error("the local recogniser failed: {0}")]
    Failed(String),
    #[error("the local recogniser's answer could not be read: {0}")]
    Malformed(String),
    #[error("the audio could not be prepared for the local recogniser: {0}")]
    Audio(String),
}

/// One word as the CLI reports it.
///
/// `speaker` is absent when the run was not diarized, and is a bare index
/// rather than a name — the CLI has no idea who anyone is.
#[derive(Debug, Clone, Deserialize)]
pub struct Word {
    pub word: String,
    pub start: f64,
    pub end: f64,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub speaker: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CliOutput {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub words: Vec<Word>,
}

/// Turn a speaker index into the spelling the rest of the app understands.
///
/// `merge::is_generic` matches "speaker" followed by digits and nothing else.
/// A bare index left as `1` would read as a real name, and folding in a Teams
/// transcript would then refuse to replace it with the person who actually
/// spoke.
fn speaker_label(index: i64) -> String {
    format!("Speaker {}", if index < 1 { 1 } else { index })
}

/// The longest silence that stays inside one segment.
///
/// Words carry timings but no segment boundaries, so they have to be grouped.
/// Speaker changes are the obvious break; this catches the other one — the
/// same person picking up again after a pause, which reads as two thoughts
/// rather than one very long sentence.
pub const SEGMENT_GAP_SECS: f64 = 1.0;

/// Group words into segments by speaker and by pause.
pub fn segments_from_words(words: &[Word]) -> Vec<TranscriptionSegment> {
    let mut out: Vec<TranscriptionSegment> = Vec::new();

    for w in words {
        let text = w.word.trim();
        if text.is_empty() {
            continue;
        }
        let speaker = w.speaker.map(speaker_label);

        let continues = out.last().is_some_and(|last| {
            last.speaker == speaker && w.start - last.end_time <= SEGMENT_GAP_SECS
        });

        if continues {
            let last = out.last_mut().expect("checked above");
            last.text.push(' ');
            last.text.push_str(text);
            last.end_time = w.end.max(last.end_time);
        } else {
            out.push(TranscriptionSegment {
                start_time: w.start,
                end_time: w.end.max(w.start),
                text: text.to_string(),
                speaker,
            });
        }
    }
    out
}

/// Parse the CLI's JSON into segments.
pub fn parse_output(stdout: &str) -> Result<TranscriptionResult, NemoError> {
    let parsed: CliOutput =
        serde_json::from_str(stdout.trim()).map_err(|e| NemoError::Malformed(e.to_string()))?;

    // Words are the only source of timings, so a run without them is not a
    // transcript this app can use — even when `text` is populated.
    if parsed.words.is_empty() {
        return Err(NemoError::Malformed(
            "the recogniser returned no word timings".into(),
        ));
    }

    let segments = segments_from_words(&parsed.words);
    let full_text = if parsed.text.trim().is_empty() {
        segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        parsed.text.trim().to_string()
    };

    Ok(TranscriptionResult {
        segments,
        full_text,
        language: None,
    })
}

/// Build the argument list for one transcription.
///
/// Separated from running it so the shape can be tested without a binary on
/// the machine.
pub fn command_args(wav: &Path, asr_model: &str, diar_model: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "transcribe".to_string(),
        wav.to_string_lossy().to_string(),
        "--format".to_string(),
        "json".to_string(),
        "--model".to_string(),
        asr_model.to_string(),
    ];
    // Without --diarize the CLI returns words with no speaker at all, which is
    // the one thing this backend exists to provide.
    if let Some(d) = diar_model {
        args.push("--diarize".to_string());
        args.push("--diar-model".to_string());
        args.push(d.to_string());
    }
    args
}

/// Where the runtime is.
///
/// A configured path wins; otherwise the name is left for the OS to resolve on
/// PATH, which is where the official installer puts it.
pub fn resolve_exe(configured: Option<&str>) -> PathBuf {
    match configured.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from("nemo-speech"),
    }
}

/// Transcribe a 16 kHz mono WAV.
///
/// The CLI rejects FLAC outright, so callers hand it a WAV. That costs nothing
/// during a recording — the recorder writes WAV and only compacts to FLAC
/// afterwards — and needs a decode only when re-transcribing something already
/// stored.
pub async fn transcribe(
    exe: &Path,
    wav: &Path,
    asr_model: &str,
    diar_model: Option<&str>,
) -> Result<TranscriptionResult, NemoError> {
    if !wav.exists() {
        return Err(NemoError::Audio(format!("{} does not exist", wav.display())));
    }

    let args = command_args(wav, asr_model, diar_model);
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(TIMEOUT_SECS),
        tokio::process::Command::new(exe)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    .map_err(|_| NemoError::Failed(format!("it did not finish within {TIMEOUT_SECS} seconds")))?
    .map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            NemoError::NotFound(exe.display().to_string())
        } else {
            NemoError::Spawn(e.to_string())
        }
    })?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let tail: String = err.lines().rev().take(3).collect::<Vec<_>>().join(" ");
        return Err(NemoError::Failed(if tail.trim().is_empty() {
            format!("it exited with {}", output.status)
        } else {
            tail
        }));
    }

    parse_output(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(word: &str, start: f64, end: f64, speaker: Option<i64>) -> Word {
        Word { word: word.into(), start, end, confidence: Some(1.0), speaker }
    }

    /// The real shape, copied from a run over a recorded meeting.
    const SAMPLE: &str = r#"{"file":"/tmp/x.wav","text":"Thing and he's",
        "confidence":1,"duration":20,"languages":["en"],
        "words":[{"word":"Thing","start":0.4,"end":0.56,"confidence":1,"speaker":1},
                 {"word":"and","start":0.64,"end":0.72,"confidence":1,"speaker":1},
                 {"word":"he's","start":0.72,"end":0.88,"confidence":1,"speaker":2}]}"#;

    #[test]
    fn the_cli_shape_parses() {
        let r = parse_output(SAMPLE).unwrap();
        assert_eq!(r.segments.len(), 2, "a speaker change splits the segment");
        assert_eq!(r.segments[0].text, "Thing and");
        assert_eq!(r.segments[1].text, "he's");
        assert_eq!(r.full_text, "Thing and he's");
    }

    #[test]
    fn speaker_indices_become_the_spelling_merge_understands() {
        // Left as a bare index, a placeholder reads as a real name and a Teams
        // merge would refuse to replace it with the actual person.
        let r = parse_output(SAMPLE).unwrap();
        assert_eq!(r.segments[0].speaker.as_deref(), Some("Speaker 1"));
        assert_eq!(r.segments[1].speaker.as_deref(), Some("Speaker 2"));
        for s in &r.segments {
            assert!(crate::merge::is_generic(s.speaker.as_deref().unwrap()));
        }
    }

    #[test]
    fn a_pause_by_the_same_speaker_starts_a_new_segment() {
        // Otherwise a quiet half-hour becomes one unreadable paragraph, which
        // is the complaint that produced the transcript rework in the first
        // place.
        let words = [w("one", 0.0, 0.5, Some(1)), w("two", 5.0, 5.5, Some(1))];
        let segs = segments_from_words(&words);
        assert_eq!(segs.len(), 2);
    }

    #[test]
    fn a_short_gap_stays_in_one_segment() {
        let words = [w("one", 0.0, 0.5, Some(1)), w("two", 0.9, 1.4, Some(1))];
        assert_eq!(segments_from_words(&words).len(), 1);
    }

    #[test]
    fn words_without_a_diarizer_carry_no_speaker_rather_than_a_wrong_one() {
        let words = [w("one", 0.0, 0.5, None), w("two", 0.6, 1.0, None)];
        let segs = segments_from_words(&words);
        assert_eq!(segs.len(), 1);
        assert!(segs[0].speaker.is_none());
    }

    #[test]
    fn a_run_without_word_timings_is_an_error_not_an_empty_transcript() {
        // `text` alone has no timings, so it cannot be placed on a timeline —
        // and a transcript with invented timings is worse than a failure.
        let err = parse_output(r#"{"text":"hello","words":[]}"#).unwrap_err();
        assert!(matches!(err, NemoError::Malformed(_)));
    }

    #[test]
    fn diarization_is_requested_explicitly() {
        // Without --diarize the CLI returns no speakers at all, which is the
        // entire reason this backend exists.
        let args = command_args(Path::new("/tmp/a.wav"), "nemotron-3.5", Some("sortformer"));
        assert!(args.contains(&"--diarize".to_string()));
        assert!(args.contains(&"json".to_string()));
        let plain = command_args(Path::new("/tmp/a.wav"), "nemotron-3.5", None);
        assert!(!plain.contains(&"--diarize".to_string()));
    }

    #[test]
    fn a_blank_configured_path_falls_back_to_the_name_on_PATH() {
        assert_eq!(resolve_exe(Some("   ")), PathBuf::from("nemo-speech"));
        assert_eq!(resolve_exe(None), PathBuf::from("nemo-speech"));
        assert_eq!(resolve_exe(Some("/opt/x/nemo-speech")), PathBuf::from("/opt/x/nemo-speech"));
    }

    #[test]
    fn segment_timings_never_run_backwards() {
        let words = [w("a", 9.0, 3.0, Some(1))];
        let segs = segments_from_words(&words);
        assert!(segs[0].end_time >= segs[0].start_time);
    }
}
