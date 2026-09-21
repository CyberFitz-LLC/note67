//! Transcription by an OpenAI-compatible recogniser.
//!
//! Distinct from `remote`, which speaks note67-asr's job API — submit, take a
//! job id, poll until it finishes. The OpenAI shape is a single synchronous
//! request that returns the transcript in its response, and it is what vLLM
//! and SGLang serve. Speaking it natively means any recogniser in that
//! ecosystem can be pointed at without another backend.
//!
//! The diarizing model this was built against, `MOSS-Transcribe-Diarize`,
//! returns its transcript as one string of `[start][Sxx]text[end]` runs rather
//! than as structured segments: vLLM rejects `verbose_json` for it, and the
//! bracketed form is the model's own canonical output. So the parsing lives
//! here rather than in serde.

use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;

use super::transcriber::{TranscriptionResult, TranscriptionSegment};

/// How long to wait for one transcription.
///
/// A single request rather than a job to poll, so the whole transcription has
/// to fit inside it. Measured at roughly three times faster than real time on
/// a workstation GPU, so an hour of audio lands in about twenty minutes; this
/// is generous enough to survive a much slower box without abandoning work
/// that would have finished.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(90 * 60);

/// What a diarized speaker label is called once it reaches the rest of the app.
///
/// The model emits `S01`, `S02`. Those are rewritten to `Speaker 1`, `Speaker
/// 2` because `merge::is_generic` recognises that spelling and only that one —
/// left as `S01`, a placeholder would read as a real name, and merging a Teams
/// transcript would refuse to replace it with the actual person.
fn speaker_label(raw: &str) -> Option<String> {
    let digits = raw.trim_start_matches(['S', 's']).trim_start_matches('0');
    let n: u32 = if digits.is_empty() { 0 } else { digits.parse().ok()? };
    Some(format!("Speaker {}", if n == 0 { 1 } else { n }))
}

#[derive(Debug, Error)]
pub enum OpenAiError {
    #[error("the transcription service could not be reached: {0}")]
    Unreachable(String),
    #[error("the transcription service returned {status}: {body}")]
    Rejected { status: u16, body: String },
    #[error("the service's answer could not be read: {0}")]
    Malformed(String),
    /// The transcript stopped short of the audio. Worth its own variant: a
    /// truncated transcript looks entirely plausible, and silently keeping one
    /// would lose the back half of a meeting without anything looking wrong.
    #[error("the transcript covers only {covered_s:.0}s of {audio_s:.0}s of audio")]
    Truncated { covered_s: f64, audio_s: f64 },
}

#[derive(Debug, Deserialize)]
struct TranscriptionResponse {
    text: String,
}

/// Pull `[start][Sxx]text[end]` runs out of the model's transcript.
///
/// Anything that does not match is skipped rather than guessed at. A partial
/// run at the end of a truncated response is not a segment with unknown
/// timings, it is evidence the answer was cut off, and `coverage` is what
/// notices that.
pub fn parse_transcript(text: &str) -> Vec<TranscriptionSegment> {
    let re = regex::Regex::new(r"\[(\d+(?:\.\d+)?)\]\[([Ss]\d+)\]([^\[]*)\[(\d+(?:\.\d+)?)\]")
        .expect("static pattern");

    re.captures_iter(text)
        .filter_map(|c| {
            let start: f64 = c.get(1)?.as_str().parse().ok()?;
            let end: f64 = c.get(4)?.as_str().parse().ok()?;
            let body = c.get(3)?.as_str().trim();
            if body.is_empty() {
                return None;
            }
            Some(TranscriptionSegment {
                start_time: start,
                end_time: end.max(start),
                text: body.to_string(),
                speaker: speaker_label(c.get(2)?.as_str()),
            })
        })
        .collect()
}

/// How much of the audio the transcript actually reaches, 0.0..=1.0.
///
/// The model's own `generation_config.json` caps output at 5120 tokens, which
/// is a third of what a busy twenty minutes needs. Past that it stops mid-
/// meeting and returns a perfectly well-formed partial transcript. Nothing in
/// the response says so; only the last timestamp does.
pub fn coverage(segments: &[TranscriptionSegment], audio_secs: f64) -> f64 {
    if audio_secs <= 0.0 {
        return 1.0;
    }
    segments
        .last()
        .map(|s| (s.end_time / audio_secs).clamp(0.0, 1.0))
        .unwrap_or(0.0)
}

/// Below this, the transcript is treated as truncated rather than complete.
///
/// A recogniser that stops early still ends on a sentence, so there is no
/// shape to detect — only the gap between the last timestamp and the length of
/// the file. Five percent of a thirty minute meeting is ninety seconds, which
/// is more than trailing silence and less than a real omission.
pub const MIN_COVERAGE: f64 = 0.95;

/// `POST /v1/audio/transcriptions`.
///
/// `max_completion_tokens` is sent on every request and deliberately not
/// optional: without it the model's own generation config caps output at 5120
/// tokens and quietly truncates anything longer.
#[allow(clippy::too_many_arguments)]
pub async fn transcribe(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
    model: &str,
    audio: Vec<u8>,
    filename: &str,
    audio_secs: f64,
    max_completion_tokens: u32,
) -> Result<TranscriptionResult, OpenAiError> {
    let url = format!(
        "{}/v1/audio/transcriptions",
        base_url.trim_end_matches('/')
    );

    let part = reqwest::multipart::Part::bytes(audio)
        .file_name(filename.to_string())
        .mime_str("application/octet-stream")
        .map_err(|e| OpenAiError::Malformed(e.to_string()))?;

    let form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", model.to_string())
        // `verbose_json` is rejected by vLLM for this model; `json` returns the
        // model's canonical bracketed transcript, which parse_transcript reads.
        .text("response_format", "json")
        .text("temperature", "0")
        .text("max_completion_tokens", max_completion_tokens.to_string());

    let mut request = client.post(&url).multipart(form).timeout(REQUEST_TIMEOUT);
    if let Some(key) = api_key.filter(|k| !k.trim().is_empty()) {
        request = request.bearer_auth(key.trim());
    }

    let response = request
        .send()
        .await
        .map_err(|e| OpenAiError::Unreachable(e.to_string()))?;

    let status = response.status();
    if !status.is_success() {
        return Err(OpenAiError::Rejected {
            status: status.as_u16(),
            body: response.text().await.unwrap_or_default(),
        });
    }

    let body: TranscriptionResponse = response
        .json()
        .await
        .map_err(|e| OpenAiError::Malformed(e.to_string()))?;

    let segments = parse_transcript(&body.text);
    let covered = coverage(&segments, audio_secs);
    if covered < MIN_COVERAGE {
        return Err(OpenAiError::Truncated {
            covered_s: segments.last().map(|s| s.end_time).unwrap_or(0.0),
            audio_s: audio_secs,
        });
    }

    let full_text = segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");

    Ok(TranscriptionResult {
        segments,
        full_text,
        language: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real excerpt, shape and all, from a MOSS run over a recorded meeting.
    const SAMPLE: &str = "[0.48][S01]Welcome everyone[1.66][12.26][S02]The new \
                          transcription pipeline is ready for evaluation[13.81]";

    #[test]
    fn the_bracketed_form_parses_into_segments() {
        let segs = parse_transcript(SAMPLE);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].start_time, 0.48);
        assert_eq!(segs[0].end_time, 1.66);
        assert_eq!(segs[0].text, "Welcome everyone");
        assert_eq!(segs[1].text, "The new transcription pipeline is ready for evaluation");
    }

    #[test]
    fn speaker_labels_are_rewritten_to_the_spelling_merge_understands() {
        // `merge::is_generic` matches "speaker" followed by digits and nothing
        // else. Left as S01 a placeholder reads as a real name, and folding in
        // a Teams transcript would then refuse to replace it with the person
        // who was actually speaking.
        let segs = parse_transcript(SAMPLE);
        assert_eq!(segs[0].speaker.as_deref(), Some("Speaker 1"));
        assert_eq!(segs[1].speaker.as_deref(), Some("Speaker 2"));
        for s in &segs {
            assert!(crate::merge::is_generic(s.speaker.as_deref().unwrap()));
        }
    }

    #[test]
    fn double_digit_speakers_survive_the_rewrite() {
        let segs = parse_transcript("[0.0][S12]hello[1.0]");
        assert_eq!(segs[0].speaker.as_deref(), Some("Speaker 12"));
        assert!(crate::merge::is_generic("Speaker 12"));
    }

    #[test]
    fn empty_runs_are_dropped_rather_than_kept_as_blank_segments() {
        let segs = parse_transcript("[0.0][S01]   [1.0][2.0][S01]real[3.0]");
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "real");
    }

    #[test]
    fn text_that_is_not_the_bracketed_form_yields_nothing() {
        // Better than guessing at timings. A caller sees zero segments and
        // reports a failure, rather than a transcript with invented timestamps.
        assert!(parse_transcript("just a plain sentence with no markers").is_empty());
    }

    #[test]
    fn coverage_notices_a_transcript_that_stops_early() {
        // The model's generation_config caps output at 5120 tokens and then
        // returns a well-formed partial transcript. The only evidence is the
        // last timestamp against the length of the file.
        let segs = parse_transcript("[0.0][S01]start[5.0][400.0][S01]middle[410.0]");
        assert!(coverage(&segs, 1262.0) < MIN_COVERAGE);
        assert!(coverage(&segs, 420.0) >= MIN_COVERAGE);
    }

    #[test]
    fn coverage_of_nothing_is_zero_not_complete() {
        assert_eq!(coverage(&[], 100.0), 0.0);
    }

    #[test]
    fn an_end_before_its_start_is_clamped_rather_than_inverted() {
        let segs = parse_transcript("[9.0][S01]backwards[3.0]");
        assert_eq!(segs[0].start_time, 9.0);
        assert_eq!(segs[0].end_time, 9.0);
    }
}
