//! Running the two panes alongside a meeting.
//!
//! Reads the transcript the app is already writing, decides when a pass is
//! worth making, calls the model, and emits what came back. It owns no
//! transcript of its own and writes nothing into one: briefs and suggestions
//! are generated text, and the transcript is the record of what was said.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Mutex;

use crate::assist::config::{Assist, MemorySource};
use crate::assist::memory;
use crate::assist::passes::{self, Line, Suggestions};
use crate::assist::schedule::{Cadence, Decision};
use crate::assist::trigger;
use crate::commands::ai::AiState;
use crate::db::Database;

/// How often the transcript is checked for new material.
///
/// Cheap — a database read — so it can be frequent. The expensive decisions are
/// made by the cadence and the trigger, not by this.
const TICK: Duration = Duration::from_secs(5);

/// How often the brief is rewritten.
const BRIEF_INTERVAL: Duration = Duration::from_secs(90);

/// The floor between suggestion passes, however talkative the room is.
const SUGGESTION_INTERVAL: Duration = Duration::from_secs(30);

/// How much recent conversation a suggestion pass is shown.
const RECENT_LINES: usize = 12;

/// What a pane is showing, and how current it is.
#[derive(Debug, Clone, Serialize)]
pub struct AssistUpdate {
    pub note_id: String,
    /// The running brief, when this update carries one.
    pub brief: Option<String>,
    pub questions_open: Vec<String>,
    pub options: Vec<AssistOption>,
    /// A reply that could not be read as structured options, shown as it came.
    pub raw: Option<String>,
    /// End time, in seconds into the meeting, of the last transcript this saw.
    ///
    /// Shown rather than a spinner. "As of 14:32" against a clock reading 14:34
    /// is what a reader needs to judge whether to trust the pane, and it is
    /// exactly what a spinner hides.
    pub as_of_seconds: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssistOption {
    pub label: String,
    pub angle: String,
}

/// Live assistance for one note.
#[derive(Default)]
pub struct AssistState {
    running: Arc<Mutex<Option<String>>>,
}

impl AssistState {
    pub async fn running_note(&self) -> Option<String> {
        self.running.lock().await.clone()
    }

    async fn set_running(&self, note_id: Option<String>) {
        *self.running.lock().await = note_id;
    }
}

/// Start the panes for a note.
pub async fn start(
    app: AppHandle,
    note_id: String,
    assist: Assist,
) -> Result<(), String> {
    let Assist::On { memory: source } = assist else {
        return Err("Live assistance is switched off in Settings.".to_string());
    };

    {
        let state = app.state::<AssistState>();
        if let Some(existing) = state.running_note().await {
            return Err(format!("Live assistance is already running for {existing}."));
        }
        state.set_running(Some(note_id.clone())).await;
    }

    tokio::spawn(run(app, note_id, source));
    Ok(())
}

/// Stop the panes.
pub async fn stop(app: &AppHandle) {
    app.state::<AssistState>().set_running(None).await;
}

async fn run(app: AppHandle, note_id: String, source: Option<MemorySource>) {
    // Read each pass rather than captured once, so changing the focus mid-
    // meeting takes effect on the next pass instead of the next meeting —
    // which is when someone realises what they actually want watched for.
    let focus_now = |app: &AppHandle| -> String {
        app.state::<Database>()
            .get_setting(crate::assist::config::FOCUS_KEY)
            .ok()
            .flatten()
            .unwrap_or_default()
    };

    let http = reqwest::Client::new();
    let mut brief_cadence = Cadence::new(BRIEF_INTERVAL);
    let mut suggest_cadence = Cadence::new(SUGGESTION_INTERVAL);
    let mut ticker = tokio::time::interval(TICK);

    // Where the brief has read up to. Everything after this is what the next
    // brief is shown, which is what keeps it incremental.
    let mut briefed_to: i64 = 0;
    let mut brief: Option<String> = None;

    loop {
        ticker.tick().await;

        {
            let state = app.state::<AssistState>();
            if state.running_note().await.as_deref() != Some(note_id.as_str()) {
                break;
            }
        }

        let segments = {
            let db = app.state::<Database>();
            match db.get_transcript_segments(&note_id) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[assist] could not read the transcript: {e}");
                    continue;
                }
            }
        };
        if segments.is_empty() {
            continue;
        }

        let as_of = segments.last().map(|s| s.end_time).unwrap_or(0.0);
        let newest_id = segments.last().map(|s| s.id).unwrap_or(0);
        let lines: Vec<Line> = segments
            .iter()
            .map(|s| Line {
                speaker: s.speaker.clone().unwrap_or_else(|| "Others".to_string()),
                text: s.text.clone(),
            })
            .collect();

        if newest_id > briefed_to {
            brief_cadence.mark_dirty();
        }

        // The brief: steady, incremental, whatever is being said.
        if brief_cadence.decide(Instant::now()) == Decision::Run {
            let unseen: Vec<Line> = segments
                .iter()
                .filter(|s| s.id > briefed_to)
                .map(|s| Line {
                    speaker: s.speaker.clone().unwrap_or_else(|| "Others".to_string()),
                    text: s.text.clone(),
                })
                .collect();

            if !unseen.is_empty() {
                let prompt = passes::brief_prompt(brief.as_deref(), &unseen, &focus_now(&app));
                match generate(&app, &prompt, 0.3).await {
                    Ok(text) => {
                        brief = Some(text.clone());
                        briefed_to = newest_id;
                        emit(
                            &app,
                            AssistUpdate {
                                note_id: note_id.clone(),
                                brief: Some(text),
                                questions_open: Vec::new(),
                                options: Vec::new(),
                                raw: None,
                                as_of_seconds: as_of,
                            },
                        );
                    }
                    Err(e) => eprintln!("[assist] brief pass failed: {e}"),
                }
            }
            brief_cadence.finished();
        }

        // Suggestions: only when the others said something worth answering.
        let recent_theirs: Vec<&Line> = lines
            .iter()
            .rev()
            .take(RECENT_LINES)
            .filter(|l| l.speaker != "You")
            .collect();
        let theirs_text: String = recent_theirs
            .iter()
            .rev()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");

        if trigger::worth_suggesting(&theirs_text) {
            suggest_cadence.mark_dirty();
        }

        if suggest_cadence.decide(Instant::now()) == Decision::Run {
            let recent: Vec<Line> = lines.iter().rev().take(RECENT_LINES).rev().cloned().collect();
            let mine: Vec<Line> = lines.iter().filter(|l| l.speaker == "You").cloned().collect();

            let memories = match &source {
                Some(m) => {
                    memory::recall(&http, &m.base_url, &m.bank, &theirs_text, 600).await
                }
                None => Vec::new(),
            };

            let prompt = passes::suggestion_prompt(&recent, &mine, &memories, &focus_now(&app));
            match generate(&app, &prompt, 0.4).await {
                Ok(reply) => {
                    let parsed: Suggestions = passes::parse_suggestions(&reply);
                    emit(
                        &app,
                        AssistUpdate {
                            note_id: note_id.clone(),
                            brief: None,
                            questions_open: parsed.questions_open,
                            options: parsed
                                .options
                                .into_iter()
                                .map(|o| AssistOption {
                                    label: o.label,
                                    angle: o.angle,
                                })
                                .collect(),
                            raw: parsed.raw_fallback,
                            as_of_seconds: as_of,
                        },
                    );
                }
                Err(e) => eprintln!("[assist] suggestion pass failed: {e}"),
            }
            suggest_cadence.finished();
        }
    }

    // Released here, not only in stop(). The loop also exits on its own when
    // the recording ends, and leaving the flag set made the next meeting
    // refuse to start with "already running" for a note that finished hours
    // ago — with nothing a user could press to clear it.
    app.state::<AssistState>().set_running(None).await;
    println!("[assist] stopped for {note_id}");
}

async fn generate(app: &AppHandle, prompt: &str, temperature: f32) -> Result<String, String> {
    let ai = app.state::<AiState>();
    let model = ai
        .selected_model
        .lock()
        .await
        .clone()
        .ok_or("no model selected")?;
    ai.client()
        .await
        .generate(&model, prompt, temperature, None)
        .await
        .map_err(|e| e.to_string())
}

fn emit(app: &AppHandle, update: AssistUpdate) {
    let _ = app.emit("assist-update", update);
}
