//! Checking a transcript against its own history, without asking anyone.
//!
//! `verify_chain` proves the versions form an unbroken sequence — that none was
//! removed, reordered or substituted. It says nothing about the transcript
//! sitting in the database right now, which could have been altered underneath
//! a chain that still verifies perfectly against itself.
//!
//! This is the check a receipt is actually *for*: the content that was attested
//! is still the content that is here. It needs no node, no token and no
//! network, and it keeps working long after any of those are reachable.

use serde::Serialize;

use note67_canonical::{content_hash, CanonicalSegment, TranscriptVersion};

/// What the current transcript is, relative to what was recorded about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum Verification {
    /// No transcript, so nothing to check. An ordinary state, not a fault.
    Empty,
    /// A transcript exists but no version was ever recorded for it. Possible
    /// only for notes that predate the chain.
    Untracked,
    /// The current content is exactly this version.
    #[serde(rename_all = "camelCase")]
    Matches {
        version: u32,
        /// True when a node signed this version. A match against an unattested
        /// version still proves the content has not changed since it was
        /// recorded — it just has nobody's signature behind it.
        attested: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        receipt_hash: Option<String>,
        /// True when this is the newest version. A match against an older one
        /// means the transcript went backwards, which is its own kind of wrong.
        is_latest: bool,
    },
    /// The content matches no recorded version. Something changed it outside
    /// the app, because every change made through the app appends a version.
    #[serde(rename_all = "camelCase")]
    Altered {
        expected_hash: String,
        actual_hash: String,
        latest_version: u32,
    },
}

/// Compare the transcript in hand against every version recorded for it.
///
/// Checks all versions rather than only the newest: matching an older one is a
/// different failure from matching none, and saying "altered" for a transcript
/// that is merely stale would send someone hunting for tampering that did not
/// happen.
pub fn verify_current(
    segments: &[CanonicalSegment],
    versions: &[TranscriptVersion],
) -> Verification {
    if segments.is_empty() && versions.is_empty() {
        return Verification::Empty;
    }
    let Some(latest) = versions.last() else {
        return Verification::Untracked;
    };

    let actual = content_hash(segments);

    if let Some(found) = versions.iter().find(|v| v.content_hash == actual) {
        return Verification::Matches {
            version: found.version,
            attested: found.receipt_hash.is_some(),
            receipt_hash: found.receipt_hash.clone(),
            is_latest: found.version == latest.version,
        };
    }

    Verification::Altered {
        expected_hash: latest.content_hash.clone(),
        actual_hash: actual,
        latest_version: latest.version,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use note67_canonical::{next_version, Origin, Reason};

    fn seg(text: &str) -> CanonicalSegment {
        CanonicalSegment {
            start_ms: 0,
            end_ms: 1000,
            speaker: Some("You".into()),
            text: text.into(),
        }
    }

    fn version_of(
        segments: &[CanonicalSegment],
        previous: Option<&TranscriptVersion>,
    ) -> TranscriptVersion {
        next_version(
            previous,
            segments,
            Origin::Recorded,
            Reason::Initial,
            "2026-08-14T00:00:00Z".into(),
        )
        .expect("content differs from the previous version")
    }

    #[test]
    fn a_note_with_nothing_in_it_is_not_a_fault() {
        assert_eq!(verify_current(&[], &[]), Verification::Empty);
    }

    #[test]
    fn a_transcript_with_no_versions_is_untracked() {
        // Notes that predate the chain. Not tampering — just unrecorded.
        assert_eq!(
            verify_current(&[seg("hello")], &[]),
            Verification::Untracked
        );
    }

    #[test]
    fn unchanged_content_matches_its_version() {
        let segments = [seg("morning everyone")];
        let v1 = version_of(&segments, None);
        match verify_current(&segments, &[v1]) {
            Verification::Matches {
                version, is_latest, attested, ..
            } => {
                assert_eq!(version, 1);
                assert!(is_latest);
                assert!(!attested, "nothing has signed it");
            }
            other => panic!("expected a match, got {other:?}"),
        }
    }

    #[test]
    fn an_attested_version_reports_its_receipt() {
        let segments = [seg("morning everyone")];
        let mut v1 = version_of(&segments, None);
        v1.receipt_hash = Some("dd12b56d".into());

        match verify_current(&segments, &[v1]) {
            Verification::Matches {
                attested,
                receipt_hash,
                ..
            } => {
                assert!(attested);
                assert_eq!(receipt_hash.as_deref(), Some("dd12b56d"));
            }
            other => panic!("expected a match, got {other:?}"),
        }
    }

    #[test]
    fn content_changed_outside_the_app_is_detected() {
        // The check this module exists for. Every change made through the app
        // appends a version, so content matching no version means something
        // edited the database directly.
        let original = [seg("we agreed to ship on friday")];
        let v1 = version_of(&original, None);
        let tampered = [seg("we agreed to ship on monday")];

        match verify_current(&tampered, &[v1.clone()]) {
            Verification::Altered {
                expected_hash,
                actual_hash,
                latest_version,
            } => {
                assert_eq!(expected_hash, v1.content_hash);
                assert_ne!(actual_hash, v1.content_hash);
                assert_eq!(latest_version, 1);
            }
            other => panic!("expected alteration, got {other:?}"),
        }
    }

    #[test]
    fn a_single_character_is_enough_to_notice() {
        let original = [seg("ship on friday")];
        let v1 = version_of(&original, None);
        let tampered = [seg("ship on Friday")];
        assert!(matches!(
            verify_current(&tampered, &[v1]),
            Verification::Altered { .. }
        ));
    }

    #[test]
    fn matching_an_older_version_is_reported_as_stale_not_altered() {
        // A different failure from tampering, and calling it tampering would
        // send someone hunting for something that did not happen.
        let first = [seg("first")];
        let v1 = version_of(&first, None);
        let second = [seg("second")];
        let v2 = version_of(&second, Some(&v1));

        match verify_current(&first, &[v1, v2]) {
            Verification::Matches {
                version, is_latest, ..
            } => {
                assert_eq!(version, 1);
                assert!(!is_latest, "content is behind the newest version");
            }
            other => panic!("expected a stale match, got {other:?}"),
        }
    }

    #[test]
    fn an_emptied_transcript_is_altered_not_empty() {
        // Deleting every segment is the most complete alteration there is, and
        // must not read as "nothing to check".
        let segments = [seg("something was said")];
        let v1 = version_of(&segments, None);
        assert!(matches!(
            verify_current(&[], &[v1]),
            Verification::Altered { .. }
        ));
    }

    #[test]
    fn reordering_segments_is_detected() {
        // Order carries meaning in a transcript, so it is part of the content.
        let ordered = [seg("first thing"), seg("second thing")];
        let v1 = version_of(&ordered, None);
        let swapped = [seg("second thing"), seg("first thing")];
        assert!(matches!(
            verify_current(&swapped, &[v1]),
            Verification::Altered { .. }
        ));
    }

    #[test]
    fn the_verdict_serializes_for_the_ui() {
        let segments = [seg("hello")];
        let v1 = version_of(&segments, None);
        let v = serde_json::to_value(verify_current(&segments, &[v1])).unwrap();
        assert_eq!(v["status"], "matches");
        assert_eq!(v["version"], 1);
        assert_eq!(v["attested"], false);
        assert_eq!(v["isLatest"], true);
    }
}
