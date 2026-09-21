//! Screenshots pasted into a meeting.
//!
//! A slide or a shared screen is often the most information-dense thing in a
//! call, and it is exactly what a transcript cannot capture. Keeping the image
//! against the moment it appeared means it can be read alongside what was being
//! said at the time.
//!
//! **What a vision model reads out of an image never enters the transcript.**
//! The transcript is what was said; it is what the chain hashes and what a
//! receipt attests. Folding model-generated text into it would produce an
//! attested record of words nobody spoke. Extracted text lives on the
//! screenshot, and reaches the assistant as context — clearly from an image.

use base64::Engine;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::commands::ai::AiState;
use crate::db::Database;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Screenshot {
    pub id: i64,
    pub note_id: String,
    pub file_path: String,
    pub captured_at_ms: i64,
    pub caption: Option<String>,
    pub extracted_text: Option<String>,
    pub created_at: String,
}

/// What a vision model is asked about a meeting screenshot.
///
/// Written for slides and shared screens rather than photographs: the useful
/// answer is the content, not a description of the picture. It is also told to
/// say when it cannot read something, because a confidently invented figure in
/// a sales call is worse than a gap.
const EXTRACT_PROMPT: &str = "\
This is a screenshot taken during a meeting — most often a slide, a document, a \
dashboard or a shared screen.

Write out its content so someone who did not see it can use it:

- Transcribe all readable text, keeping headings, lists and table structure.
- Give the actual figures in any chart, and say what it is showing.
- Describe diagrams in terms of what they mean, not how they look.

Do not speculate about anything you cannot read — say it is unclear instead. Do \
not add commentary or summarise. If the image holds no readable content, say so \
in one line.";

/// Store a pasted image against a point in the meeting.
///
/// `captured_at_ms` is the offset into the note, the same clock transcript
/// segments use, so the image sits in the timeline where it appeared.
#[tauri::command]
pub fn add_screenshot(
    app: AppHandle,
    db: State<Database>,
    note_id: String,
    image_base64: String,
    captured_at_ms: i64,
    caption: Option<String>,
) -> Result<Screenshot, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(image_base64.trim())
        .map_err(|e| format!("that did not decode as an image: {e}"))?;

    // Refuse anything that is not actually a PNG or JPEG rather than writing it
    // and failing later when something tries to display it.
    let mime = sniff_image(&bytes).ok_or_else(|| {
        "only PNG and JPEG images can be pasted — that did not look like either".to_string()
    })?;
    let extension = if mime == "image/png" { "png" } else { "jpg" };

    let dir = crate::commands::images::attachments_dir(&app, &note_id)?;
    let filename = format!("screenshot_{}.{}", chrono::Utc::now().timestamp_millis(), extension);
    let path = dir.join(&filename);
    std::fs::write(&path, &bytes).map_err(|e| format!("could not save the image: {e}"))?;

    db.add_screenshot(
        &note_id,
        &path.to_string_lossy(),
        captured_at_ms,
        caption.as_deref(),
    )
    .map_err(|e| {
        // The row is the thing that makes the file findable; without it the
        // image is litter.
        let _ = std::fs::remove_file(&path);
        e.to_string()
    })
}

/// Identify an image from its own bytes rather than trusting a file name.
fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else {
        None
    }
}

#[tauri::command]
pub fn list_screenshots(db: State<Database>, note_id: String) -> Result<Vec<Screenshot>, String> {
    db.list_screenshots(&note_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_screenshot(db: State<Database>, id: i64) -> Result<(), String> {
    // The file goes with the row. A screenshot is not a recording — it can be
    // pasted again, and leaving orphans on disk is the mistake recordings
    // already made.
    if let Ok(Some(shot)) = db.get_screenshot(id) {
        let _ = std::fs::remove_file(&shot.file_path);
    }
    db.delete_screenshot(id).map_err(|e| e.to_string())
}

/// Read an image with the configured vision model and keep what it found.
#[tauri::command]
pub async fn extract_screenshot_text(
    db: State<'_, Database>,
    ai: State<'_, AiState>,
    id: i64,
) -> Result<String, String> {
    let shot = db
        .get_screenshot(id)
        .map_err(|e| e.to_string())?
        .ok_or("that screenshot no longer exists")?;

    let bytes = std::fs::read(&shot.file_path)
        .map_err(|e| format!("could not read the image back: {e}"))?;
    let mime = sniff_image(&bytes).unwrap_or("image/png");

    let model = ai
        .selected_model
        .lock()
        .await
        .clone()
        .ok_or("No model selected. Please select a model first.")?;

    let text = ai
        .client()
        .await
        .generate_with_image(&model, EXTRACT_PROMPT, &bytes, mime, 0.2)
        .await
        .map_err(|e| e.to_string())?;

    db.set_screenshot_text(id, &text).map_err(|e| e.to_string())?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_png_is_recognised_from_its_bytes() {
        assert_eq!(
            sniff_image(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0]),
            Some("image/png")
        );
    }

    #[test]
    fn a_jpeg_is_recognised_from_its_bytes() {
        assert_eq!(sniff_image(&[0xff, 0xd8, 0xff, 0xe0, 0]), Some("image/jpeg"));
    }

    #[test]
    fn anything_else_is_refused() {
        // Sniffed rather than taken on trust, so a clipboard carrying something
        // else is rejected before it is written to disk and shown as an image.
        assert_eq!(sniff_image(b"GIF89a"), None);
        assert_eq!(sniff_image(b"<html>"), None);
        assert_eq!(sniff_image(&[]), None);
    }

    #[test]
    fn the_prompt_asks_for_content_and_forbids_invention() {
        // Both matter in a sales call: a figure read off a slide is useful, and
        // a figure the model made up is a liability.
        assert!(EXTRACT_PROMPT.contains("Transcribe all readable text"));
        assert!(EXTRACT_PROMPT.contains("Do not speculate"));
    }
}
