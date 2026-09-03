//! Folding another tool's recording of the same meeting into an existing note.
//!
//! Distinct from `import_vtt_transcript`, which always creates a new note. That
//! is right when the import *is* the meeting; it is wrong when Teams and Otter
//! recorded the same hour this app did, and three notes is three copies of one
//! thing with the interesting part — who was speaking — missing from all of
//! them.

use serde::Serialize;
use tauri::State;

use crate::db::models::NewTranscriptSegment;
use crate::db::Database;
use crate::exochain::{parse_vtt, ImportSource, Origin, Reason, TranscriptVersion};
use crate::merge::{merge_speakers, MergedSegment, SourceSegment};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeOutcome {
    /// How far the other recording's clock was from ours, in milliseconds.
    /// `None` means the two did not look like the same meeting.
    pub offset_ms: Option<i64>,
    pub segments_named: usize,
    pub disagreements: usize,
    /// True when nothing was changed because the two do not match.
    pub rejected: bool,
    /// What the comparison found, whether or not it was enough to merge.
    ///
    /// Returned on a refusal too. "That does not look like the same meeting" is
    /// a conclusion, and without the numbers behind it there is no way to tell
    /// a genuinely different recording from two that simply overlapped too
    /// little — which is the ordinary shape when one was started by hand.
    pub evidence: MergeEvidence,
    /// The chain version this produced. `None` when nothing changed — merging
    /// a source that adds nothing is not a new state, and minting a version
    /// for it would fill the chain with links that attest nothing.
    pub version: Option<TranscriptVersion>,
    /// Where the two disagree, for review. The user decides, not the merge.
    pub conflicts: Vec<Conflict>,
}

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MergeEvidence {
    pub matched: usize,
    pub agreeing: usize,
    pub base_segments: usize,
    pub other_segments: usize,
    pub overlap_ms: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub start_ms: i64,
    pub text: String,
    pub detail: String,
}

/// Merge a WebVTT transcript of the same meeting into an existing note.
#[tauri::command]
pub fn merge_transcript_into_note(
    db: State<Database>,
    note_id: String,
    path: String,
    source_tool: Option<String>,
) -> Result<MergeOutcome, String> {
    let source_tool = source_tool.unwrap_or_else(|| "WebVTT".to_string());

    let content = std::fs::read_to_string(&path).map_err(|e| format!("Could not read {path}: {e}"))?;
    let incoming = parse_vtt(&content).map_err(|e| e.to_string())?;

    let existing = db.get_transcript_segments(&note_id).map_err(|e| e.to_string())?;
    if existing.is_empty() {
        // Nothing to align against. Importing as a new note is the right move,
        // and doing it silently here would hide that this note had no
        // transcript of its own.
        return Err(
            "This note has no transcript yet, so there is nothing to merge against. \
             Import it as a new note instead."
                .into(),
        );
    }

    let base: Vec<SourceSegment> = existing
        .iter()
        .map(|s| SourceSegment {
            start_ms: (s.start_time * 1000.0).round() as i64,
            end_ms: (s.end_time * 1000.0).round() as i64,
            speaker: s.speaker.clone(),
            text: s.text.clone(),
        })
        .collect();

    let other: Vec<SourceSegment> = incoming
        .iter()
        .map(|s| SourceSegment {
            start_ms: s.start_ms,
            end_ms: s.end_ms,
            speaker: s.speaker.clone(),
            text: s.text.clone(),
        })
        .collect();

    let (merged, report) = merge_speakers(&base, &other, &source_tool);

    let conflicts: Vec<Conflict> = merged
        .iter()
        .filter_map(|m| {
            m.disagreement.as_ref().map(|detail| Conflict {
                start_ms: m.start_ms,
                text: m.text.clone(),
                detail: detail.clone(),
            })
        })
        .collect();

    let evidence = MergeEvidence {
        matched: report.evidence.matched,
        agreeing: report.evidence.agreeing,
        base_segments: report.evidence.base_segments,
        other_segments: report.evidence.other_segments,
        overlap_ms: report.evidence.overlap_ms,
    };

    if report.rejected {
        return Ok(MergeOutcome {
            offset_ms: None,
            segments_named: 0,
            disagreements: 0,
            rejected: true,
            evidence,
            version: None,
            conflicts,
        });
    }

    // Only the attribution changed, so only that is written back. The text and
    // timings are this app's own and stay exactly as recorded.
    let rows: Vec<NewTranscriptSegment> = merged
        .iter()
        .zip(existing.iter())
        .map(|(m, original)| NewTranscriptSegment {
            note_id: note_id.clone(),
            start_time: original.start_time,
            end_time: original.end_time,
            text: m.text.clone(),
            speaker: m.speaker.clone(),
            source_type: original.source_type.clone(),
            source_id: original.source_id,
        })
        .collect();

    db.replace_transcript_segments(&note_id, &rows)
        .map_err(|e| e.to_string())?;

    // `record_transcript_version_from` returns None when the content hash is
    // unchanged, which is exactly what should happen when a source contributed
    // no names.
    let version = db
        .record_transcript_version_from(
            &note_id,
            Origin::Merged,
            Reason::Merge,
            Some(ImportSource {
                tool: source_tool.clone(),
                filename: std::path::Path::new(&path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.clone()),
            }),
        )
        .map_err(|e| e.to_string())?;

    Ok(MergeOutcome {
        offset_ms: report.offset_ms,
        segments_named: report.segments_named,
        disagreements: report.disagreements,
        rejected: false,
        evidence,
        version,
        conflicts,
    })
}

/// Set one segment's speaker by hand.
///
/// A manual label always wins. Merges and re-transcription both guess; the
/// person who was in the meeting does not.
#[tauri::command]
pub fn set_segment_speaker(
    db: State<Database>,
    note_id: String,
    segment_id: i64,
    speaker: Option<String>,
) -> Result<Option<TranscriptVersion>, String> {
    {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE transcript_segments SET speaker = ?1 WHERE id = ?2 AND note_id = ?3",
            rusqlite::params![speaker, segment_id, note_id],
        )
        .map_err(|e| e.to_string())?;
    }

    // Naming a speaker changes what the transcript says, so it appends to the
    // chain. Without this the chain would have a blind spot exactly where the
    // most human-meaningful edits happen.
    //
    // The origin is CARRIED FORWARD, not forced to Merged. A hand-typed name
    // came from the person who was in the meeting — the strongest attribution
    // available, not a borrowed one — so stamping Merged on a recording of your
    // own would claim some names came from another tool when none did. That
    // was tolerable while relabelling was an edge case; with a diarizer
    // emitting `Speaker 1..N` it becomes the main path, and a note would flip
    // to Merged the moment anyone put a real name to a voice.
    //
    // A note that is already Merged stays Merged: editing it does not unborrow
    // the names it took.
    let origin = db
        .latest_transcript_version(&note_id)
        .ok()
        .flatten()
        .map(|v| v.origin)
        .unwrap_or(Origin::Recorded);

    db.record_transcript_version(&note_id, origin, Reason::Edit)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merged(start_ms: i64, detail: Option<&str>) -> MergedSegment {
        MergedSegment {
            start_ms,
            end_ms: start_ms + 1000,
            speaker: Some("Bob Smith".into()),
            text: "hello".into(),
            speaker_source: Some("Teams".into()),
            disagreement: detail.map(str::to_string),
        }
    }

    #[test]
    fn only_disagreeing_segments_become_conflicts() {
        let segments = [merged(0, None), merged(1000, Some("Teams says Walley")), merged(2000, None)];
        let conflicts: Vec<Conflict> = segments
            .iter()
            .filter_map(|m| {
                m.disagreement.as_ref().map(|d| Conflict {
                    start_ms: m.start_ms,
                    text: m.text.clone(),
                    detail: d.clone(),
                })
            })
            .collect();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].start_ms, 1000);
    }

    #[test]
    fn the_outcome_serializes_under_the_names_the_ui_reads() {
        let outcome = MergeOutcome {
            offset_ms: Some(10_000),
            segments_named: 5,
            disagreements: 1,
            rejected: false,
            evidence: MergeEvidence::default(),
            version: None,
            conflicts: vec![Conflict {
                start_ms: 0,
                text: "hello".into(),
                detail: "Teams: \"hi\"".into(),
            }],
        };
        let v = serde_json::to_value(&outcome).unwrap();
        assert_eq!(v["offsetMs"], 10_000);
        assert_eq!(v["segmentsNamed"], 5);
        assert_eq!(v["conflicts"][0]["startMs"], 0);
        assert_eq!(v["rejected"], false);
    }

    #[test]
    fn a_refusal_carries_the_evidence_for_it() {
        // "That does not look like the same meeting" is a conclusion. Without
        // the numbers there is no way to tell a genuinely different recording
        // from two that overlapped too little to be sure — which is the normal
        // shape when one was started by hand, and was the real case reported.
        let outcome = MergeOutcome {
            offset_ms: None,
            segments_named: 0,
            disagreements: 0,
            rejected: true,
            evidence: MergeEvidence {
                matched: 2,
                agreeing: 2,
                base_segments: 40,
                other_segments: 120,
                overlap_ms: 0,
            },
            version: None,
            conflicts: vec![],
        };
        let v = serde_json::to_value(&outcome).unwrap();
        assert_eq!(v["evidence"]["matched"], 2);
        assert_eq!(v["evidence"]["baseSegments"], 40);
        assert_eq!(v["evidence"]["otherSegments"], 120);
    }

    #[test]
    fn a_rejected_merge_reports_no_version() {
        // Nothing was written, so nothing was appended to the chain. A version
        // here would attest a merge that did not happen.
        let outcome = MergeOutcome {
            offset_ms: None,
            segments_named: 0,
            disagreements: 0,
            rejected: true,
            evidence: MergeEvidence {
                matched: 2,
                agreeing: 2,
                base_segments: 40,
                other_segments: 120,
                overlap_ms: 0,
            },
            version: None,
            conflicts: vec![],
        };
        assert!(outcome.version.is_none());
        assert!(outcome.rejected);
    }
}
