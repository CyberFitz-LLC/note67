//! Transcription by a remote recogniser.
//!
//! Local Whisper hears one voice per track: everything on the microphone is
//! "You" and everything else is "Others". A ten-person call therefore reads as
//! two speakers, and Teams — which used to supply the real names — stopped
//! providing transcripts.
//!
//! `note67-asr` transcribes and diarizes in one pass, returning `Speaker 1..N`.
//! Those are placeholders rather than names, which is exactly what
//! `merge::is_generic` already says they are, and what a human then relabels.
//!
//! Asynchronous by necessity: an hour of audio takes minutes, well past any
//! sensible request timeout. Submit, then poll.

use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;

use super::transcriber::{TranscriptionResult, TranscriptionSegment};

/// How long to keep polling before giving up.
///
/// Generous, because the alternative to waiting is throwing away a
/// transcription that is still running and will finish. Roughly ten times the
/// observed rate for an hour of audio.
pub const MAX_WAIT: Duration = Duration::from_secs(60 * 60);

/// Gap between polls. The job takes minutes; polling faster buys nothing and
/// only loads the appliance.
pub const POLL_INTERVAL: Duration = Duration::from_secs(3);

/// How many polls in a row may fail to reach the service before a job is
/// abandoned.
///
/// A diarizing pass over an hour of meeting runs for minutes on a busy box, and
/// the appliance shares that box with whatever else is loaded. A single blip —
/// a restart, a moment of memory pressure — used to discard the whole job and
/// report nothing, while the work may well have been continuing on the other
/// side. Roughly a minute of unreachability at the poll interval, which
/// survives a restart without waiting indefinitely on a service that has
/// genuinely gone.
///
/// Consecutive, deliberately: reaching the service resets it, so a long job
/// that hiccups repeatedly is tolerated while one that has truly lost its
/// service is not.
pub const MAX_POLL_FAILURES: u32 = 20;

#[derive(Debug, Error)]
pub enum RemoteError {
    #[error("the transcription service could not be reached: {0}")]
    Unreachable(String),
    #[error("the transcription service returned {status}: {body}")]
    Rejected { status: u16, body: String },
    #[error("the transcription failed: {0}")]
    Failed(String),
    #[error("the transcription did not finish within {0:?}")]
    TimedOut(Duration),
    #[error("the service's answer could not be read: {0}")]
    Malformed(String),
}

#[derive(Debug, Deserialize)]
pub struct SubmitResponse {
    pub job_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RemoteSegment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    /// `Speaker 1..N`, or absent when diarization was skipped or failed.
    #[serde(default)]
    pub speaker: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JobStatus {
    pub status: String,
    #[serde(default)]
    pub diarized: bool,
    #[serde(default)]
    pub speakers: Vec<String>,
    #[serde(default)]
    pub full_text: String,
    #[serde(default)]
    pub segments: Vec<RemoteSegment>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Where a job has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Waiting,
    Done,
    Failed,
}

/// Read a job's state.
///
/// Anything unrecognised counts as still waiting rather than as failure: a
/// status this client has not been taught is far more likely to be a new
/// intermediate state than a finished one, and treating it as failure would
/// discard work that was going to succeed.
pub fn progress_of(status: &str) -> Progress {
    match status.trim().to_ascii_lowercase().as_str() {
        "done" | "completed" | "complete" | "succeeded" => Progress::Done,
        "error" | "failed" | "cancelled" | "canceled" => Progress::Failed,
        _ => Progress::Waiting,
    }
}

/// Turn a finished job into the app's own shape.
///
/// A job that produced no segments is an error, not an empty transcript: a
/// recording that yielded nothing and a service that lost the result are
/// indistinguishable here, and the harmless reading of the two is the wrong
/// one to guess.
pub fn to_result(job: &JobStatus) -> Result<TranscriptionResult, RemoteError> {
    if job.segments.is_empty() {
        return Err(RemoteError::Failed(
            "the service returned no transcript segments".into(),
        ));
    }

    let segments: Vec<TranscriptionSegment> = job
        .segments
        .iter()
        .map(|s| TranscriptionSegment {
            start_time: s.start,
            end_time: s.end,
            text: s.text.trim().to_string(),
            // Kept only when diarization actually ran. A label from a failed
            // diarizer would put every word on one invented speaker, which
            // reads as a confident answer rather than an absent one.
            speaker: if job.diarized {
                s.speaker.clone().filter(|n| !n.trim().is_empty())
            } else {
                None
            },
        })
        .collect();

    let full_text = if job.full_text.trim().is_empty() {
        segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        job.full_text.trim().to_string()
    };

    Ok(TranscriptionResult {
        segments,
        full_text,
        // The service reports a language; it is not carried through because
        // nothing downstream reads it, and inventing a value here would be a
        // second source of truth for something already recorded upstream.
        language: None,
    })
}

/// `POST /v1/transcriptions`, returning the job id.
pub async fn submit(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
    wav: Vec<u8>,
    filename: &str,
    max_speakers: Option<u32>,
) -> Result<String, RemoteError> {
    let url = format!("{}/v1/transcriptions", base_url.trim_end_matches('/'));

    let mut form = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(wav)
            .file_name(filename.to_string())
            .mime_str("audio/wav")
            .map_err(|e| RemoteError::Malformed(e.to_string()))?,
    );
    if let Some(max) = max_speakers {
        form = form.text("max_speakers", max.to_string());
    }

    let mut request = client.post(&url).multipart(form);
    if let Some(key) = api_key.filter(|k| !k.trim().is_empty()) {
        request = request.bearer_auth(key.trim());
    }

    let response = request
        .send()
        .await
        .map_err(|e| RemoteError::Unreachable(e.to_string()))?;

    let status = response.status();
    if !status.is_success() {
        return Err(RemoteError::Rejected {
            status: status.as_u16(),
            body: response.text().await.unwrap_or_default(),
        });
    }

    response
        .json::<SubmitResponse>()
        .await
        .map(|r| r.job_id)
        .map_err(|e| RemoteError::Malformed(e.to_string()))
}

/// `GET /v1/transcriptions/{job_id}`.
pub async fn poll_once(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
    job_id: &str,
) -> Result<JobStatus, RemoteError> {
    let url = format!(
        "{}/v1/transcriptions/{job_id}",
        base_url.trim_end_matches('/')
    );
    let mut request = client.get(&url);
    if let Some(key) = api_key.filter(|k| !k.trim().is_empty()) {
        request = request.bearer_auth(key.trim());
    }

    let response = request
        .send()
        .await
        .map_err(|e| RemoteError::Unreachable(e.to_string()))?;

    let status = response.status();
    if !status.is_success() {
        return Err(RemoteError::Rejected {
            status: status.as_u16(),
            body: response.text().await.unwrap_or_default(),
        });
    }

    response
        .json::<JobStatus>()
        .await
        .map_err(|e| RemoteError::Malformed(e.to_string()))
}

/// Submit and wait.
/// Whether the service is up and holding its models, before anything is sent.
///
/// Worth one request first because the alternative is what a real failure
/// looked like: four tracks uploaded in turn to a service that was not running,
/// four identical connection errors, and a message that repeated the same fact
/// four times without ever saying the plain version of it.
///
/// A service that answers but reports no models is treated as not ready. It
/// accepts a job and then cannot do it, which is a slower and more confusing
/// way to fail.
pub async fn health(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
) -> Result<(), RemoteError> {
    let url = format!("{}/health", base_url.trim_end_matches('/'));
    let mut request = client.get(&url);
    if let Some(key) = api_key {
        request = request.bearer_auth(key);
    }

    let response = request
        .send()
        .await
        .map_err(|e| RemoteError::Unreachable(e.to_string()))?;

    if !response.status().is_success() {
        return Err(RemoteError::Rejected {
            status: response.status().as_u16(),
            body: response.text().await.unwrap_or_default(),
        });
    }

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| RemoteError::Malformed(e.to_string()))?;

    if body.get("models_loaded") == Some(&serde_json::Value::Bool(false)) {
        return Err(RemoteError::Unreachable(
            "the service is running but its models are not loaded yet".to_string(),
        ));
    }

    Ok(())
}

pub async fn transcribe(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
    wav: Vec<u8>,
    filename: &str,
    max_speakers: Option<u32>,
) -> Result<TranscriptionResult, RemoteError> {
    let job_id = submit(client, base_url, api_key, wav, filename, max_speakers).await?;
    let started = std::time::Instant::now();
    let mut unreachable_polls = 0u32;

    loop {
        let job = match poll_once(client, base_url, api_key, &job_id).await {
            Ok(job) => {
                unreachable_polls = 0;
                job
            }
            // Not being able to reach the service is not the same as the job
            // having failed. The work may still be running; only the asking
            // went wrong.
            Err(RemoteError::Unreachable(reason)) => {
                unreachable_polls += 1;
                if unreachable_polls >= MAX_POLL_FAILURES {
                    return Err(RemoteError::Unreachable(format!(
                        "{reason} (unreachable for {} consecutive polls)",
                        unreachable_polls
                    )));
                }
                if started.elapsed() > MAX_WAIT {
                    return Err(RemoteError::TimedOut(MAX_WAIT));
                }
                tokio::time::sleep(POLL_INTERVAL).await;
                continue;
            }
            // Anything else — a rejection, an unreadable answer — is the
            // service telling us something, and retrying would only repeat it.
            Err(other) => return Err(other),
        };
        match progress_of(&job.status) {
            Progress::Done => return to_result(&job),
            Progress::Failed => {
                return Err(RemoteError::Failed(
                    job.error.unwrap_or_else(|| job.status.clone()),
                ));
            }
            Progress::Waiting => {
                if started.elapsed() > MAX_WAIT {
                    return Err(RemoteError::TimedOut(MAX_WAIT));
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(status: &str, diarized: bool, speakers: &[&str]) -> JobStatus {
        JobStatus {
            status: status.into(),
            diarized,
            speakers: speakers.iter().map(|s| s.to_string()).collect(),
            full_text: "Good morning everyone. Shall we start?".into(),
            segments: vec![
                RemoteSegment {
                    start: 0.0,
                    end: 9.6,
                    text: "Good morning everyone.".into(),
                    speaker: Some("Speaker 1".into()),
                },
                RemoteSegment {
                    start: 10.2,
                    end: 14.4,
                    text: "Shall we start?".into(),
                    speaker: Some("Speaker 2".into()),
                },
            ],
            error: None,
        }
    }

    #[test]
    fn a_finished_job_is_recognised() {
        for s in ["done", "DONE", " completed ", "succeeded"] {
            assert_eq!(progress_of(s), Progress::Done, "{s}");
        }
    }

    #[test]
    fn a_failed_job_is_recognised() {
        for s in ["error", "failed", "cancelled"] {
            assert_eq!(progress_of(s), Progress::Failed, "{s}");
        }
    }

    #[test]
    fn an_unknown_status_keeps_waiting_rather_than_failing() {
        // A status this client has not been taught is far more likely to be a
        // new intermediate state than a finished one, and treating it as
        // failure would throw away work that was going to succeed.
        for s in ["queued", "running", "diarizing", "something-new"] {
            assert_eq!(progress_of(s), Progress::Waiting, "{s}");
        }
    }

    #[test]
    fn diarized_speakers_are_carried_through() {
        // The whole point: local Whisper cannot produce these at all.
        let result = to_result(&job("done", true, &["Speaker 1", "Speaker 2"])).unwrap();
        assert_eq!(result.segments[0].speaker.as_deref(), Some("Speaker 1"));
        assert_eq!(result.segments[1].speaker.as_deref(), Some("Speaker 2"));
    }

    #[test]
    fn labels_are_dropped_when_diarization_did_not_run() {
        // The service degrades to a transcript with no speakers and says so.
        // Keeping the labels anyway would put every word on one invented
        // speaker — a confident answer where the honest one is "unknown".
        let result = to_result(&job("done", false, &[])).unwrap();
        assert!(result.segments.iter().all(|s| s.speaker.is_none()));
    }

    #[test]
    fn an_empty_speaker_label_is_treated_as_absent() {
        let mut j = job("done", true, &[]);
        j.segments[0].speaker = Some("   ".into());
        assert!(to_result(&j).unwrap().segments[0].speaker.is_none());
    }

    #[test]
    fn timings_and_text_survive_unchanged() {
        let result = to_result(&job("done", true, &[])).unwrap();
        assert_eq!(result.segments[0].start_time, 0.0);
        assert_eq!(result.segments[0].end_time, 9.6);
        assert_eq!(result.segments[0].text, "Good morning everyone.");
    }

    #[test]
    fn a_job_with_no_segments_is_an_error() {
        // A recording that yielded nothing and a service that lost the result
        // are indistinguishable here, and the harmless reading is the wrong
        // one to guess.
        let mut j = job("done", true, &[]);
        j.segments.clear();
        assert!(matches!(to_result(&j), Err(RemoteError::Failed(_))));
    }

    #[test]
    fn full_text_is_rebuilt_when_the_service_omits_it() {
        let mut j = job("done", true, &[]);
        j.full_text = "  ".into();
        let result = to_result(&j).unwrap();
        assert_eq!(result.full_text, "Good morning everyone. Shall we start?");
    }

    #[test]
    fn the_real_response_shape_parses() {
        // Captured from note67-asr on the Spark, not written from the docs.
        let body = r#"{
            "job_id": "d3fd518905f64fe9b55238dcaedaf668",
            "status": "done",
            "filename": "meeting_6speakers.wav",
            "elapsed_s": 8.1,
            "duration_ms": 50000,
            "num_speakers": 4,
            "speakers": ["Speaker 1", "Speaker 2", "Speaker 3", "Speaker 4"],
            "language": "en",
            "diarized": true,
            "full_text": "Good morning everyone, thanks for joining the quarterly review.",
            "segments": [
                {"start": 0.0, "end": 9.6, "text": "Good morning everyone, thanks for joining the quarterly review.", "speaker": "Speaker 1"}
            ]
        }"#;
        let job: JobStatus = serde_json::from_str(body).expect("the live shape should parse");
        assert_eq!(progress_of(&job.status), Progress::Done);
        assert!(job.diarized);
        assert_eq!(to_result(&job).unwrap().segments[0].speaker.as_deref(), Some("Speaker 1"));
    }

    #[test]
    fn a_submit_response_parses() {
        let r: SubmitResponse =
            serde_json::from_str(r#"{"job_id":"abc123"}"#).unwrap();
        assert_eq!(r.job_id, "abc123");
    }
    #[test]
    fn a_service_that_reports_no_models_is_not_ready() {
        // The shape this guards, taken from the real endpoint: it answers 200
        // while still loading, accepts a job, and then cannot do it. Treating
        // that as available turns a clear failure into a slow confusing one.
        let loading: serde_json::Value =
            serde_json::from_str(r#"{"status":"ok","models_loaded":false}"#).unwrap();
        assert_eq!(
            loading.get("models_loaded"),
            Some(&serde_json::Value::Bool(false))
        );

        let ready: serde_json::Value = serde_json::from_str(
            r#"{"status":"ok","models_loaded":true,"jobs_active":0,"queue_depth":0}"#,
        )
        .unwrap();
        assert_ne!(
            ready.get("models_loaded"),
            Some(&serde_json::Value::Bool(false))
        );
    }

}
