//! What the panes are doing, when they are not showing anything.
//!
//! A pane that says "Listening…" for ten minutes has three quite different
//! things it could mean — there is no transcript to read, a pass is running, or
//! every pass has failed — and it says none of them. That happened in a real
//! meeting, and every explanation was in an `eprintln!` that a release build
//! never shows anyone.
//!
//! This is the same silence that produced an empty summary block, a
//! retranscribe that appeared to do nothing, and a recogniser falling behind
//! without saying so. Written as its own module because after four times the
//! pattern is worth naming: **when a feature has nothing to show, it should say
//! which kind of nothing it has.**

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    /// Running, but the transcript is empty so far.
    NoTranscript,
    /// A pass is in flight.
    Thinking,
    /// A pass completed and the panes hold its result.
    Ready,
    /// A pass failed. The reason is the part that was missing.
    Failed { reason: String },
}

impl Status {
    /// What to show a reader, in words rather than a state name.
    pub fn message(&self) -> String {
        match self {
            // Names the likeliest cause, because "no transcript" invites the
            // question this answers: assistance reads the transcript, so if
            // there is none, transcription is what to look at.
            Status::NoTranscript => {
                "Nothing transcribed yet — assistance reads the transcript, so check that \
                 transcription is running."
                    .to_string()
            }
            Status::Thinking => "Working on it…".to_string(),
            Status::Ready => "Up to date.".to_string(),
            Status::Failed { reason } => format!("The model did not answer: {reason}"),
        }
    }

    /// Whether this is worth showing as a problem rather than as progress.
    pub fn is_problem(&self) -> bool {
        matches!(self, Status::Failed { .. } | Status::NoTranscript)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_transcript_says_what_to_look_at() {
        // The real report: ten minutes of "Listening…" while the pane had no
        // idea whether the transcript was empty, the model was failing, or a
        // pass was merely slow.
        let m = Status::NoTranscript.message();
        assert!(m.contains("transcription is running"));
        assert!(Status::NoTranscript.is_problem());
    }

    #[test]
    fn a_failure_carries_its_reason() {
        // The reason used to go to eprintln, which a release build shows nobody.
        let m = Status::Failed {
            reason: "connection refused".into(),
        }
        .message();
        assert!(m.contains("connection refused"));
    }

    #[test]
    fn working_and_ready_are_not_problems() {
        assert!(!Status::Thinking.is_problem());
        assert!(!Status::Ready.is_problem());
    }

    #[test]
    fn the_wire_form_names_the_state() {
        let json = serde_json::to_string(&Status::Failed {
            reason: "timeout".into(),
        })
        .unwrap();
        assert!(json.contains("\"state\":\"failed\""));
        assert!(json.contains("timeout"));
    }
}
