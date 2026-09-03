//! Starting and stopping live meeting assistance.

use tauri::{AppHandle, Manager, State};

use crate::assist::config::{self, Assist};
use crate::assist::passes;
use crate::assist::runner::{self, AssistState};
use crate::commands::ai::AiState;
use crate::db::Database;

/// What starting assistance produced, including whether it was attested.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AssistStarted {
    pub running: bool,
    /// The receipt for this session, when the node minted one.
    pub receipt: Option<String>,
    /// Why there is no receipt, when there is not. Shown rather than swallowed:
    /// a session running unattested is a fact the user should be able to see.
    pub attestation_note: Option<String>,
}

fn read_config(db: &Database) -> Assist {
    let get = |key: &str| db.get_setting(key).ok().flatten();
    config::resolve(
        &get(config::ENABLED_KEY).unwrap_or_default(),
        get(config::HINDSIGHT_URL_KEY).as_deref(),
        get(config::HINDSIGHT_BANK_KEY).as_deref(),
    )
}

/// Begin assisting a meeting.
///
/// A receipt is minted once, here, for the session as a whole — not per pass.
/// Continuous summarisation is one governed activity that starts when it is
/// switched on; a receipt for every ninety-second pass would be a chain nobody
/// could read, and would say nothing the first one does not.
#[tauri::command]
pub async fn start_assist(
    app: AppHandle,
    db: State<'_, Database>,
    note_id: String,
) -> Result<AssistStarted, String> {
    let assist = read_config(&db);
    if assist == Assist::Off {
        return Err("Live assistance is switched off in Settings.".to_string());
    }

    // A model must be configured before anything is sent anywhere.
    {
        let ai = app.state::<AiState>();
        if ai.selected_model.lock().await.is_none() {
            return Err("No model is selected — choose one in Settings first.".to_string());
        }
    }

    // Attest the session before it starts, so the record exists for the whole
    // of what it describes rather than for the part that had happened when
    // someone remembered to ask.
    let (receipt, attestation_note) =
        match crate::commands::exochain::attest_assist_session(&app, &note_id).await {
            Ok(hash) => (Some(hash), None),
            Err(reason) => (None, Some(reason)),
        };

    runner::start(app.clone(), note_id, assist).await?;

    Ok(AssistStarted {
        running: true,
        receipt,
        attestation_note,
    })
}

#[tauri::command]
pub async fn stop_assist(app: AppHandle) -> Result<(), String> {
    runner::stop(&app).await;
    Ok(())
}

#[tauri::command]
pub async fn assist_running(app: AppHandle) -> Result<Option<String>, String> {
    Ok(app.state::<AssistState>().running_note().await)
}

/// Expand one suggested option into something to say.
///
/// A second, focused pass rather than a longer first one. Having been told
/// which of several directions was chosen is a much better prompt than "suggest
/// a response", and it costs nothing until a button is actually pressed.
#[tauri::command]
pub async fn expand_assist_option(
    app: AppHandle,
    db: State<'_, Database>,
    note_id: String,
    label: String,
    angle: String,
) -> Result<String, String> {
    let segments = db
        .get_transcript_segments(&note_id)
        .map_err(|e| e.to_string())?;

    let recent: Vec<passes::Line> = segments
        .iter()
        .rev()
        .take(12)
        .rev()
        .map(|s| passes::Line {
            speaker: s.speaker.clone().unwrap_or_else(|| "Others".to_string()),
            text: s.text.clone(),
        })
        .collect();

    let prompt = passes::follow_up_prompt(&recent, &label, &angle);

    let ai = app.state::<AiState>();
    let model = ai
        .selected_model
        .lock()
        .await
        .clone()
        .ok_or("No model is selected.")?;
    ai.client()
        .await
        .generate(&model, &prompt, 0.4, None)
        .await
        .map_err(|e| e.to_string())
}
