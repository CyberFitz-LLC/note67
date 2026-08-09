//! Importing transcripts produced by other tools.
//!
//! An imported transcript joins the version chain like any recorded one, but
//! its origin is `Imported` and it names the tool and file it came from. That
//! distinction is the point: for a recording Note67 observed the whole pipeline,
//! whereas for an import all that can ever be attested is that this content
//! arrived at a given time and has not changed since. A receipt that blurred
//! the two would claim something nobody checked.

use chrono::Utc;
use serde::Serialize;
use tauri::State;

use crate::db::models::NewTranscriptSegment;
use crate::db::Database;
use crate::exochain::{parse_vtt, ImportSource, Origin, Reason, TranscriptVersion};

/// What an import produced.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub note_id: String,
    pub title: String,
    pub segment_count: usize,
    /// The chain's first version. Always present: an import always creates a
    /// note, so there is nothing for it to be identical to.
    pub version: Option<TranscriptVersion>,
    /// Speakers found in the file, for the UI to show what was recognised.
    pub speakers: Vec<String>,
}

/// Import a WebVTT transcript as a new note.
///
/// A new note rather than merging into an existing one: merging would
/// interleave content Note67 produced with content it did not, and the chain
/// records a single origin per version, so the result could not honestly be
/// labelled either way.
/// Reads the file here rather than in the webview: the frontend's filesystem
/// scope is deliberately limited to the app's own data directory, and widening
/// it so a picker could read Downloads would hand the webview general file
/// access for one feature.
#[tauri::command]
pub fn import_vtt_transcript(
    db: State<Database>,
    path: String,
    title: String,
    source_tool: Option<String>,
) -> Result<ImportResult, String> {
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Could not read {path}: {e}"))?;
    let segments = parse_vtt(&content).map_err(|e| e.to_string())?;

    // The name the user knows the file by. The full path is theirs and does not
    // belong in a transcript's recorded provenance.
    let filename = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.clone());

    let title = if title.trim().is_empty() {
        // Fall back to the filename without its extension, which for a Teams
        // export is usually the meeting name.
        filename
            .rsplit_once('.')
            .map(|(stem, _)| stem)
            .unwrap_or(&filename)
            .trim()
            .to_string()
    } else {
        title.trim().to_string()
    };
    let title = if title.is_empty() {
        "Imported transcript".to_string()
    } else {
        title
    };

    let note_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    {
        // Scoped so the connection lock is released before the Database
        // helpers below take it again — the mutex is not reentrant.
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO notes (id, title, description, participants, started_at, ended_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            (
                &note_id,
                &title,
                &Some(format!("Imported from {filename}")),
                &None::<String>,
                &now,
                // Ended immediately: an import is a finished meeting, and
                // leaving ended_at null would present it as still recording.
                &now,
                &now,
                &now,
            ),
        )
        .map_err(|e| e.to_string())?;
    }

    let rows: Vec<NewTranscriptSegment> = segments
        .iter()
        .map(|s| NewTranscriptSegment {
            note_id: note_id.clone(),
            start_time: s.start_ms as f64 / 1000.0,
            end_time: s.end_ms as f64 / 1000.0,
            text: s.text.clone(),
            speaker: s.speaker.clone(),
            // No audio backs an imported transcript, so there is no source
            // recording to point at.
            source_type: None,
            source_id: None,
        })
        .collect();

    db.add_transcript_segments_batch(&rows)
        .map_err(|e| e.to_string())?;

    let version = db
        .record_transcript_version_from(
            &note_id,
            Origin::Imported,
            Reason::Import,
            Some(ImportSource {
                tool: source_tool.unwrap_or_else(|| "WebVTT".to_string()),
                filename: filename.clone(),
            }),
        )
        .map_err(|e| e.to_string())?;

    let mut speakers: Vec<String> = segments.iter().filter_map(|s| s.speaker.clone()).collect();
    speakers.sort();
    speakers.dedup();

    Ok(ImportResult {
        note_id,
        title,
        segment_count: segments.len(),
        version,
        speakers,
    })
}
