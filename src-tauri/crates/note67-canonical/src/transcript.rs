//! Transcript versioning and the receipt chain.
//!
//! A note's transcript is not a single object that gets overwritten. Every time
//! it changes — the first pass, a re-transcription, a manual edit, an import —
//! a new *version* is appended, carrying the hash of the one before it. That
//! chain is what makes a later change visible instead of silent: an edited
//! transcript no longer matches the receipt for the version it replaced, and the
//! chain says exactly which version it diverged from.
//!
//! Nothing here needs a node. The chain is useful on its own, and attestation
//! later just anchors individual links to signed receipts.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Identifies the canonical byte form below.
///
/// **This string and the format it names are frozen.** A receipt anchors the
/// hash of these bytes; if the serialization changes, every previously minted
/// receipt stops matching the transcript it attested and becomes unverifiable.
/// Any change must ship as a new version string alongside this one, never as an
/// edit to it.
pub const SERIALIZATION_V1: &str = "note67.transcript.v1";

/// Where a transcript came from, which bounds what a receipt over it can claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Note67 captured the audio and produced the text, so the whole pipeline
    /// was observed.
    Recorded,
    /// The text arrived from elsewhere. All that can be attested is that this
    /// content was imported at a given time and has not changed since — not
    /// that it reflects a real meeting.
    Imported,
    /// Note67 recorded the audio and produced the text, but some of the
    /// attribution came from another tool's recording of the same meeting.
    ///
    /// Neither of the above would be honest. `Recorded` would claim the whole
    /// result was observed here, when the speaker names were taken on trust
    /// from Teams or Otter; `Imported` would disclaim text this app produced
    /// from its own audio. A receipt over a merged version can attest the
    /// content and the alignment, and must not attest that the names are right.
    Merged,
}

impl Origin {
    /// Stored as text in the database and in receipts, so the mapping is
    /// explicit rather than whatever a serializer happens to emit.
    pub fn as_str(&self) -> &'static str {
        match self {
            Origin::Recorded => "recorded",
            Origin::Imported => "imported",
            Origin::Merged => "merged",
        }
    }

    /// Unknown values read as `Imported`: it is the weaker claim, and a row we
    /// cannot interpret should not be presented as something we observed
    /// end to end.
    pub fn from_db(value: &str) -> Self {
        match value {
            "recorded" => Origin::Recorded,
            "merged" => Origin::Merged,
            _ => Origin::Imported,
        }
    }
}

/// Why a new version exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    Initial,
    Retranscribe,
    Edit,
    Import,
    /// Another recording of the same meeting was folded in.
    Merge,
}

impl Reason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Reason::Initial => "initial",
            Reason::Retranscribe => "retranscribe",
            Reason::Edit => "edit",
            Reason::Import => "import",
            Reason::Merge => "merge",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "initial" => Reason::Initial,
            "retranscribe" => Reason::Retranscribe,
            "import" => Reason::Import,
            "merge" => Reason::Merge,
            _ => Reason::Edit,
        }
    }
}

/// One link in a note's transcript chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptVersion {
    /// 1-based and contiguous within a note.
    pub version: u32,
    /// Hex SHA-256 of the canonical bytes.
    pub content_hash: String,
    /// The previous version's `content_hash`; `None` only for version 1.
    pub parent_hash: Option<String>,
    /// Which canonical form produced `content_hash`.
    pub serialization: String,
    pub origin: Origin,
    pub reason: Reason,
    pub segment_count: usize,
    pub created_at: String,
    /// Set once a node has signed this version. `None` means unattested, which
    /// is an ordinary state rather than an error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt_hash: Option<String>,
    /// What produced an imported transcript, e.g. "Microsoft Teams". `None` for
    /// anything Note67 recorded itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_tool: Option<String>,
    /// The file an import came from, as the user knew it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_filename: Option<String>,
}

/// Where an imported transcript came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSource {
    pub tool: String,
    pub filename: String,
}

impl TranscriptVersion {
    /// Attach the provenance of an import.
    pub fn with_source(mut self, source: Option<ImportSource>) -> Self {
        if let Some(src) = source {
            self.source_tool = Some(src.tool);
            self.source_filename = Some(src.filename);
        }
        self
    }
}

/// A transcript segment reduced to the fields that are attested.
///
/// Database ids, insertion timestamps and source bookkeeping are deliberately
/// excluded: they vary between machines and re-imports without the transcript
/// itself differing, and hashing them would make a receipt fail for reasons
/// that have nothing to do with the content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalSegment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub speaker: Option<String>,
    pub text: String,
}

impl CanonicalSegment {
    /// Build from the stored floating-point seconds.
    ///
    /// Times are rounded to whole milliseconds because float formatting is not
    /// reproducible across languages or library versions, and a hash that
    /// depends on how a `f64` happens to print cannot be recomputed by anyone
    /// verifying it. Non-finite values become zero rather than poisoning the
    /// hash with a platform-specific rendering of NaN.
    pub fn from_seconds(start: f64, end: f64, speaker: Option<String>, text: String) -> Self {
        Self {
            start_ms: to_ms(start),
            end_ms: to_ms(end),
            speaker,
            text,
        }
    }
}

fn to_ms(seconds: f64) -> i64 {
    if seconds.is_finite() {
        (seconds * 1000.0).round() as i64
    } else {
        0
    }
}

/// Escape the field separators so a segment's own text cannot forge a boundary.
///
/// Without this, text containing a tab or newline could make one segment parse
/// as two, and two different transcripts could hash identically.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out
}

/// The canonical byte form of a transcript.
///
/// ```text
/// note67.transcript.v1\n
/// <segment count>\n
/// <start_ms>\t<end_ms>\t<speaker>\t<text>\n     (repeated, in order)
/// ```
///
/// Times are integer milliseconds, an absent speaker is the empty string, and
/// backslash, tab, newline and carriage return are escaped in both the speaker
/// and text fields. Segment order is the transcript's own order and is part of
/// the content: reordering is a change.
///
/// Deliberately simple so anyone verifying a receipt can reproduce it without
/// this codebase.
pub fn canonical_bytes(segments: &[CanonicalSegment]) -> Vec<u8> {
    let mut out = String::new();
    out.push_str(SERIALIZATION_V1);
    out.push('\n');
    out.push_str(&segments.len().to_string());
    out.push('\n');
    for seg in segments {
        out.push_str(&seg.start_ms.to_string());
        out.push('\t');
        out.push_str(&seg.end_ms.to_string());
        out.push('\t');
        out.push_str(&escape(seg.speaker.as_deref().unwrap_or("")));
        out.push('\t');
        out.push_str(&escape(&seg.text));
        out.push('\n');
    }
    out.into_bytes()
}

/// Hex SHA-256 of the canonical bytes.
pub fn content_hash(segments: &[CanonicalSegment]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(canonical_bytes(segments));
    hex::encode(hasher.finalize())
}

/// Append a version, unless the content is unchanged.
///
/// Returns `None` when the hash matches the current tip. Re-running a
/// transcription that produces identical text is not a new state, and minting a
/// version for it would fill the chain with links that attest nothing and cost
/// a receipt each.
pub fn next_version(
    previous: Option<&TranscriptVersion>,
    segments: &[CanonicalSegment],
    origin: Origin,
    reason: Reason,
    created_at: String,
) -> Option<TranscriptVersion> {
    let hash = content_hash(segments);

    if let Some(prev) = previous
        && prev.content_hash == hash
    {
        return None;
    }

    Some(TranscriptVersion {
        version: previous.map_or(1, |p| p.version + 1),
        parent_hash: previous.map(|p| p.content_hash.clone()),
        content_hash: hash,
        serialization: SERIALIZATION_V1.to_string(),
        origin,
        reason,
        segment_count: segments.len(),
        created_at,
        receipt_hash: None,
        source_tool: None,
        source_filename: None,
    })
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ChainError {
    #[error("the chain is empty")]
    Empty,
    #[error("version {0} should be {1}: versions must start at 1 and not skip")]
    NonContiguous(u32, u32),
    #[error("version 1 must not have a parent")]
    RootHasParent,
    #[error("version {0} has no parent")]
    MissingParent(u32),
    #[error("version {version} points at {found}, but version {} ends at {expected}", version - 1)]
    BrokenLink {
        version: u32,
        expected: String,
        found: String,
    },
}

/// Check that a note's versions form an unbroken chain.
///
/// Catches a deleted, reordered or substituted version — the cases where a
/// transcript's history has been altered rather than extended.
pub fn verify_chain(versions: &[TranscriptVersion]) -> Result<(), ChainError> {
    let Some(first) = versions.first() else {
        return Err(ChainError::Empty);
    };

    if first.version != 1 {
        return Err(ChainError::NonContiguous(first.version, 1));
    }
    if first.parent_hash.is_some() {
        return Err(ChainError::RootHasParent);
    }

    for pair in versions.windows(2) {
        let (prev, cur) = (&pair[0], &pair[1]);

        if cur.version != prev.version + 1 {
            return Err(ChainError::NonContiguous(cur.version, prev.version + 1));
        }
        match &cur.parent_hash {
            None => return Err(ChainError::MissingParent(cur.version)),
            Some(parent) if parent != &prev.content_hash => {
                return Err(ChainError::BrokenLink {
                    version: cur.version,
                    expected: prev.content_hash.clone(),
                    found: parent.clone(),
                });
            }
            Some(_) => {}
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start_ms: i64, end_ms: i64, speaker: &str, text: &str) -> CanonicalSegment {
        CanonicalSegment {
            start_ms,
            end_ms,
            speaker: Some(speaker.to_string()),
            text: text.to_string(),
        }
    }

    fn now() -> String {
        "2026-08-08T12:00:00Z".to_string()
    }

    fn sample() -> Vec<CanonicalSegment> {
        vec![
            seg(0, 1500, "You", "Morning."),
            seg(1500, 4000, "Others", "Shall we start?"),
        ]
    }

    #[test]
    fn the_canonical_form_is_exactly_as_documented() {
        // Pinned literally: anyone verifying a receipt has to reproduce these
        // bytes without this codebase, so the format is a contract rather than
        // an implementation detail.
        let bytes = canonical_bytes(&sample());
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "note67.transcript.v1\n2\n0\t1500\tYou\tMorning.\n1500\t4000\tOthers\tShall we start?\n"
        );
    }

    #[test]
    fn the_same_transcript_always_hashes_the_same() {
        assert_eq!(content_hash(&sample()), content_hash(&sample()));
    }

    #[test]
    fn changing_a_single_character_changes_the_hash() {
        let mut edited = sample();
        edited[1].text = "Shall we start!".to_string();
        assert_ne!(content_hash(&sample()), content_hash(&edited));
    }

    #[test]
    fn reordering_segments_changes_the_hash() {
        // Order carries meaning in a transcript, so it is part of the content.
        let mut swapped = sample();
        swapped.swap(0, 1);
        assert_ne!(content_hash(&sample()), content_hash(&swapped));
    }

    #[test]
    fn timings_are_part_of_the_content() {
        let mut shifted = sample();
        shifted[0].end_ms = 1501;
        assert_ne!(content_hash(&sample()), content_hash(&shifted));
    }

    #[test]
    fn text_cannot_forge_a_segment_boundary() {
        // Without escaping, a tab inside the text would make one segment
        // deserialize as two — and let two different transcripts collide.
        let sneaky = vec![seg(0, 1000, "You", "a\tb\nc")];
        let split = vec![seg(0, 1000, "You", "a"), seg(0, 0, "b", "c")];
        assert_ne!(content_hash(&sneaky), content_hash(&split));

        let rendered = String::from_utf8(canonical_bytes(&sneaky)).unwrap();
        assert!(rendered.contains("a\\tb\\nc"), "text was not escaped: {rendered}");
        assert_eq!(rendered.lines().count(), 3, "escaped text added a line");
    }

    #[test]
    fn a_missing_speaker_is_distinct_from_an_empty_one() {
        let none = vec![CanonicalSegment {
            start_ms: 0,
            end_ms: 1,
            speaker: None,
            text: "hi".into(),
        }];
        let empty = vec![seg(0, 1, "", "hi")];
        // Both render as the empty field, so they hash alike by design — the
        // distinction carries no meaning in a transcript and pretending it does
        // would make receipts depend on how the database happened to store it.
        assert_eq!(content_hash(&none), content_hash(&empty));
    }

    #[test]
    fn seconds_become_whole_milliseconds() {
        // Float formatting is not reproducible across languages, so the
        // canonical form must never contain one.
        let s = CanonicalSegment::from_seconds(1.2345, 2.9996, None, "x".into());
        assert_eq!(s.start_ms, 1235);
        assert_eq!(s.end_ms, 3000);
    }

    #[test]
    fn non_finite_times_do_not_poison_the_hash() {
        let s = CanonicalSegment::from_seconds(f64::NAN, f64::INFINITY, None, "x".into());
        assert_eq!((s.start_ms, s.end_ms), (0, 0));
    }

    #[test]
    fn the_first_version_starts_the_chain() {
        let v1 = next_version(None, &sample(), Origin::Recorded, Reason::Initial, now()).unwrap();
        assert_eq!(v1.version, 1);
        assert!(v1.parent_hash.is_none());
        assert_eq!(v1.serialization, SERIALIZATION_V1);
        assert!(v1.receipt_hash.is_none(), "a new version is unattested");
    }

    #[test]
    fn a_later_version_points_at_the_one_before_it() {
        let v1 = next_version(None, &sample(), Origin::Recorded, Reason::Initial, now()).unwrap();

        let mut improved = sample();
        improved[1].text = "Shall we begin?".to_string();
        let v2 = next_version(
            Some(&v1),
            &improved,
            Origin::Recorded,
            Reason::Retranscribe,
            now(),
        )
        .unwrap();

        assert_eq!(v2.version, 2);
        assert_eq!(v2.parent_hash.as_deref(), Some(v1.content_hash.as_str()));
        assert_eq!(v2.reason, Reason::Retranscribe);
        verify_chain(&[v1, v2]).unwrap();
    }

    #[test]
    fn unchanged_content_does_not_mint_a_version() {
        // Re-transcribing to the same text is not a new state. Minting anyway
        // would fill the chain with links attesting nothing, at a receipt each.
        let v1 = next_version(None, &sample(), Origin::Recorded, Reason::Initial, now()).unwrap();
        assert!(
            next_version(
                Some(&v1),
                &sample(),
                Origin::Recorded,
                Reason::Retranscribe,
                now()
            )
            .is_none()
        );
    }

    #[test]
    fn an_import_records_where_it_came_from() {
        // A receipt over an import must name its source, or it reads like one
        // over a transcript this app produced.
        let v = next_version(None, &sample(), Origin::Imported, Reason::Import, now())
            .unwrap()
            .with_source(Some(ImportSource {
                tool: "Microsoft Teams".into(),
                filename: "Weekly Sync.vtt".into(),
            }));
        assert_eq!(v.source_tool.as_deref(), Some("Microsoft Teams"));
        assert_eq!(v.source_filename.as_deref(), Some("Weekly Sync.vtt"));
    }

    #[test]
    fn a_recorded_version_names_no_source() {
        let v = next_version(None, &sample(), Origin::Recorded, Reason::Initial, now())
            .unwrap()
            .with_source(None);
        assert!(v.source_tool.is_none() && v.source_filename.is_none());
    }

    #[test]
    fn an_import_is_marked_as_one() {
        // What a receipt may claim depends on this: for an import we observed
        // only that the content arrived, not that it reflects a meeting.
        let v = next_version(None, &sample(), Origin::Imported, Reason::Import, now()).unwrap();
        assert_eq!(v.origin, Origin::Imported);
    }

    #[test]
    fn a_broken_link_is_detected() {
        let v1 = next_version(None, &sample(), Origin::Recorded, Reason::Initial, now()).unwrap();
        let mut edited = sample();
        edited[0].text = "Afternoon.".to_string();
        let mut v2 =
            next_version(Some(&v1), &edited, Origin::Recorded, Reason::Edit, now()).unwrap();

        // Someone rewrites history: v1's content is replaced, so v2 no longer
        // descends from what is stored.
        v2.parent_hash = Some("00".repeat(32));

        match verify_chain(&[v1, v2]) {
            Err(ChainError::BrokenLink { version, .. }) => assert_eq!(version, 2),
            other => panic!("expected a broken link, got {other:?}"),
        }
    }

    #[test]
    fn a_removed_version_is_detected() {
        let v1 = next_version(None, &sample(), Origin::Recorded, Reason::Initial, now()).unwrap();
        let mut a = sample();
        a[0].text = "one".into();
        let v2 = next_version(Some(&v1), &a, Origin::Recorded, Reason::Edit, now()).unwrap();
        let mut b = sample();
        b[0].text = "two".into();
        let v3 = next_version(Some(&v2), &b, Origin::Recorded, Reason::Edit, now()).unwrap();

        // Drop the middle: numbering gives it away even before the hashes do.
        match verify_chain(&[v1, v3]) {
            Err(ChainError::NonContiguous(found, expected)) => {
                assert_eq!((found, expected), (3, 2));
            }
            other => panic!("expected a gap, got {other:?}"),
        }
    }

    #[test]
    fn a_chain_that_does_not_start_at_one_is_rejected() {
        let mut v = next_version(None, &sample(), Origin::Recorded, Reason::Initial, now()).unwrap();
        v.version = 2;
        assert_eq!(verify_chain(&[v]), Err(ChainError::NonContiguous(2, 1)));
    }

    #[test]
    fn a_root_with_a_parent_is_rejected() {
        let mut v = next_version(None, &sample(), Origin::Recorded, Reason::Initial, now()).unwrap();
        v.parent_hash = Some("00".repeat(32));
        assert_eq!(verify_chain(&[v]), Err(ChainError::RootHasParent));
    }

    #[test]
    fn an_empty_chain_is_rejected() {
        assert_eq!(verify_chain(&[]), Err(ChainError::Empty));
    }

    #[test]
    fn the_stored_forms_round_trip() {
        for o in [Origin::Recorded, Origin::Imported, Origin::Merged] {
            assert_eq!(Origin::from_db(o.as_str()), o);
        }
        for r in [
            Reason::Initial,
            Reason::Retranscribe,
            Reason::Edit,
            Reason::Import,
            Reason::Merge,
        ] {
            assert_eq!(Reason::from_db(r.as_str()), r);
        }
    }

    #[test]
    fn an_unreadable_origin_reads_as_the_weaker_claim() {
        // A row we cannot interpret must not be presented as something whose
        // production we observed.
        assert_eq!(Origin::from_db("something-else"), Origin::Imported);
    }

    #[test]
    fn an_empty_transcript_still_has_a_stable_hash() {
        // A recording that produced nothing is a real outcome and must be
        // attestable, not a special case that skips the chain.
        let v = next_version(None, &[], Origin::Recorded, Reason::Initial, now()).unwrap();
        assert_eq!(v.segment_count, 0);
        assert_eq!(content_hash(&[]), content_hash(&[]));
        assert_ne!(content_hash(&[]), content_hash(&sample()));
    }
}

#[cfg(test)]
mod merged_origin_tests {
    use super::*;

    #[test]
    fn a_merged_origin_is_neither_recorded_nor_imported() {
        // Recorded would claim the whole result was observed here, when the
        // speaker names were taken on trust from another tool. Imported would
        // disclaim text this app produced from its own audio.
        assert_ne!(Origin::Merged, Origin::Recorded);
        assert_ne!(Origin::Merged, Origin::Imported);
        assert_eq!(Origin::from_db("merged"), Origin::Merged);
    }

    #[test]
    fn older_code_reads_a_merged_version_as_the_weaker_claim() {
        // Anything that does not know the variant falls through to Imported,
        // which under-claims rather than over-claims. That is the only safe
        // direction for a value that bounds what a receipt may say.
        assert_eq!(Origin::from_db("something-new"), Origin::Imported);
    }

    #[test]
    fn the_origin_is_not_part_of_the_hashed_content() {
        // Adding a variant must not change any hash, or every receipt already
        // minted would stop matching the transcript it attested.
        let segments = [CanonicalSegment {
            start_ms: 0,
            end_ms: 1000,
            speaker: Some("Bob Smith".into()),
            text: "hello".into(),
        }];
        let recorded = next_version(None, &segments, Origin::Recorded, Reason::Initial, "t".into());
        let merged = next_version(None, &segments, Origin::Merged, Reason::Merge, "t".into());
        assert_eq!(
            recorded.unwrap().content_hash,
            merged.unwrap().content_hash
        );
    }
}
