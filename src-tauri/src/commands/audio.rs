use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::audio::{
    self, build_playback_track, is_system_audio_available, AudioDevice, RecordingPhase,
    RecordingState, SystemAudioCapture,
};
use crate::db::Database;

/// Settings key holding the pinned input device name.
pub const INPUT_DEVICE_SETTING: &str = "input_device";
/// Settings key holding the pinned playback (loopback capture) device name.
pub const OUTPUT_DEVICE_SETTING: &str = "output_device";

/// Result of dual recording containing paths to all recorded files
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DualRecordingResult {
    /// Path to the mic recording (None for listen-only / system-audio-only sessions)
    pub mic_path: Option<String>,
    /// Path to the system audio recording (only on supported platforms with permission)
    pub system_path: Option<String>,
    /// Path to the merged playback file (created after recording stops)
    pub playback_path: Option<String>,
}

pub struct AudioState {
    pub recording: Arc<RecordingState>,
    /// System audio capture instance (macOS only)
    pub system_capture: Mutex<Option<Arc<dyn SystemAudioCapture>>>,
    /// Path to the system audio recording file
    pub system_output_path: Mutex<Option<PathBuf>>,
}

impl Default for AudioState {
    fn default() -> Self {
        // Try to create system audio capture if supported
        let system_capture = crate::audio::create_system_audio_capture().ok();

        Self {
            recording: Arc::new(RecordingState::new()),
            system_capture: Mutex::new(system_capture),
            system_output_path: Mutex::new(None),
        }
    }
}

#[tauri::command]
pub fn start_recording(
    app: AppHandle,
    state: State<AudioState>,
    note_id: String,
) -> Result<String, String> {
    // Get app data directory for storing recordings
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;

    let recordings_dir = app_data_dir.join("recordings");
    std::fs::create_dir_all(&recordings_dir).map_err(|e| e.to_string())?;

    let filename = format!("{}.wav", note_id);
    let output_path = recordings_dir.join(&filename);

    audio::start_recording(state.recording.clone(), output_path.clone())
        .map_err(|e| e.to_string())?;

    Ok(output_path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn stop_recording(state: State<AudioState>) -> Result<Option<String>, String> {
    let path = audio::stop_recording(&state.recording).map_err(|e| e.to_string())?;
    Ok(path.map(|p| p.to_string_lossy().to_string()))
}

#[tauri::command]
pub fn get_recording_status(state: State<AudioState>) -> bool {
    state.recording.is_recording.load(Ordering::SeqCst)
}

#[tauri::command]
pub fn get_audio_level(state: State<AudioState>) -> f32 {
    f32::from_bits(state.recording.audio_level.load(Ordering::SeqCst))
}

/// Check if system audio capture is available on this platform
#[tauri::command]
pub fn is_system_audio_supported() -> bool {
    is_system_audio_available()
}

/// Check if the app has permission to capture system audio
#[tauri::command]
pub fn has_system_audio_permission(state: State<AudioState>) -> Result<bool, String> {
    let capture = state.system_capture.lock().map_err(|e| e.to_string())?;

    match capture.as_ref() {
        Some(cap) => cap.has_permission().map_err(|e| e.to_string()),
        None => Ok(false),
    }
}

/// Request permission to capture system audio
/// On macOS, this will trigger the system permission dialog if needed
#[tauri::command]
pub fn request_system_audio_permission(state: State<AudioState>) -> Result<bool, String> {
    let capture = state.system_capture.lock().map_err(|e| e.to_string())?;

    match capture.as_ref() {
        Some(cap) => cap.request_permission().map_err(|e| e.to_string()),
        None => Err("System audio capture not supported on this platform".to_string()),
    }
}

// ========== Microphone Permission Commands ==========

/// Check if a microphone is available on this device
#[tauri::command]
pub fn has_microphone_available() -> bool {
    use cpal::traits::HostTrait;

    let host = cpal::default_host();

    // Check if there's a default input device
    if host.default_input_device().is_some() {
        return true;
    }

    // If no default, check if there are any input devices at all
    if let Ok(devices) = host.input_devices() {
        return devices.count() > 0;
    }

    false
}

// ========== Input Device Selection ==========

/// List the input devices available for recording.
#[tauri::command]
pub fn list_audio_input_devices() -> Result<Vec<AudioDevice>, String> {
    audio::list_input_devices().map_err(|e| e.to_string())
}

/// Get the pinned input device name. `None` means "follow the system default".
#[tauri::command]
pub fn get_preferred_input_device(state: State<AudioState>) -> Option<String> {
    state.recording.get_preferred_input_device()
}

/// Pin an input device by name, or pass `None` to follow the system default.
///
/// The name is not validated against the devices present: a user who unplugs a
/// USB mic overnight should still have it pinned in the morning. Recording falls
/// back to the system default for as long as it is absent.
#[tauri::command]
pub fn set_preferred_input_device(
    device_name: Option<String>,
    state: State<AudioState>,
    db: State<Database>,
) -> Result<(), String> {
    state
        .recording
        .set_preferred_input_device(device_name)
        .map_err(|e| e.to_string())?;

    // Read back the normalised value so the database and the running state
    // cannot disagree about what "" means.
    let stored = state.recording.get_preferred_input_device();
    db.set_setting(INPUT_DEVICE_SETTING, stored.as_deref().unwrap_or(""))
        .map_err(|e| e.to_string())
}

/// Load the pinned input device from settings into the running audio state.
///
/// Called once at startup; the device is otherwise only read when a recording
/// segment begins.
pub fn restore_preferred_input_device(state: &AudioState, db: &Database) {
    match db.get_setting(INPUT_DEVICE_SETTING) {
        Ok(Some(name)) => {
            if let Err(e) = state.recording.set_preferred_input_device(Some(name)) {
                eprintln!("Failed to restore the saved input device: {}", e);
            }
        }
        Ok(None) => {}
        Err(e) => eprintln!("Failed to read the saved input device: {}", e),
    }
}

// ========== Output (playback) Device Selection ==========

/// Whether this platform lets the user choose which playback device is captured.
#[tauri::command]
pub fn is_output_device_selectable() -> bool {
    audio::is_output_device_selectable()
}

/// List the playback devices whose audio can be captured.
#[tauri::command]
pub fn list_audio_output_devices() -> Result<Vec<AudioDevice>, String> {
    audio::list_output_devices().map_err(|e| e.to_string())
}

/// Get the pinned playback device name. `None` follows the system default.
#[tauri::command]
pub fn get_preferred_output_device(state: State<AudioState>) -> Result<Option<String>, String> {
    let capture = state.system_capture.lock().map_err(|e| e.to_string())?;
    Ok(capture.as_ref().and_then(|cap| cap.get_preferred_device()))
}

/// Pin a playback device to capture system audio from.
///
/// As with the microphone, the name is not validated against what is present:
/// a docking station unplugged overnight should still be pinned in the morning.
#[tauri::command]
pub fn set_preferred_output_device(
    device_name: Option<String>,
    state: State<AudioState>,
    db: State<Database>,
) -> Result<(), String> {
    {
        let capture = state.system_capture.lock().map_err(|e| e.to_string())?;
        match capture.as_ref() {
            Some(cap) => cap.set_preferred_device(device_name).map_err(|e| e.to_string())?,
            None => return Err("System audio capture is not available on this platform".to_string()),
        }
    }

    // Read back the normalised value so the database and the running state
    // cannot disagree about what "" means.
    let stored = {
        let capture = state.system_capture.lock().map_err(|e| e.to_string())?;
        capture.as_ref().and_then(|cap| cap.get_preferred_device())
    };
    db.set_setting(OUTPUT_DEVICE_SETTING, stored.as_deref().unwrap_or(""))
        .map_err(|e| e.to_string())
}

/// Load the pinned playback device from settings into the running audio state.
pub fn restore_preferred_output_device(state: &AudioState, db: &Database) {
    let Ok(Some(name)) = db.get_setting(OUTPUT_DEVICE_SETTING) else {
        return;
    };

    let Ok(capture) = state.system_capture.lock() else {
        return;
    };
    if let Some(cap) = capture.as_ref()
        && let Err(e) = cap.set_preferred_device(Some(name))
    {
        eprintln!("Failed to restore the saved playback device: {}", e);
    }
}

/// Check if the app has microphone permission (macOS)
#[cfg(target_os = "macos")]
#[tauri::command]
pub fn has_microphone_permission() -> bool {
    use objc2::{class, msg_send};
    use objc2_foundation::NSString;

    unsafe {
        // AVAuthorizationStatus values:
        // 0 = NotDetermined, 1 = Restricted, 2 = Denied, 3 = Authorized
        let cls = class!(AVCaptureDevice);
        let media_type = NSString::from_str("soun"); // AVMediaTypeAudio = "soun"
        let status: i64 = msg_send![cls, authorizationStatusForMediaType: &*media_type];
        status == 3 // Authorized
    }
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn has_microphone_permission() -> bool {
    // On non-macOS platforms, assume permission is granted if mic is available
    has_microphone_available()
}

/// Get microphone authorization status (macOS)
/// Returns: 0 = NotDetermined, 1 = Restricted, 2 = Denied, 3 = Authorized
#[cfg(target_os = "macos")]
#[tauri::command]
pub fn get_microphone_auth_status() -> i64 {
    use objc2::{class, msg_send};
    use objc2_foundation::NSString;

    unsafe {
        let cls = class!(AVCaptureDevice);
        let media_type = NSString::from_str("soun"); // AVMediaTypeAudio
        let status: i64 = msg_send![cls, authorizationStatusForMediaType: &*media_type];
        status
    }
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn get_microphone_auth_status() -> i64 {
    // Return "Authorized" on non-macOS if mic is available
    if has_microphone_available() { 3 } else { 2 }
}

/// Request microphone permission (macOS)
/// This triggers the system permission dialog and makes the app appear in System Settings
#[cfg(target_os = "macos")]
#[tauri::command]
pub fn request_microphone_permission() -> bool {
    use objc2::{class, msg_send};
    use objc2::runtime::Bool;
    use objc2_foundation::NSString;

    unsafe {
        let cls = class!(AVCaptureDevice);
        let media_type = NSString::from_str("soun"); // AVMediaTypeAudio

        // Create a block for the completion handler
        // We use a no-op block since we'll have the user refresh the status
        let block = block2::RcBlock::new(|_granted: Bool| {
            // Permission dialog shown, user will refresh to check status
        });

        // Request access - this triggers the permission dialog
        let _: () = msg_send![cls, requestAccessForMediaType: &*media_type, completionHandler: &*block];
    }

    // Return current status after triggering the dialog
    // User should refresh to get the final result
    has_microphone_permission()
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn request_microphone_permission() -> bool {
    // On non-macOS platforms, just check if mic is available
    has_microphone_available()
}

/// Start dual recording (mic + system audio)
/// Returns paths to both recording files
#[tauri::command]
pub fn start_dual_recording(
    app: AppHandle,
    state: State<AudioState>,
    note_id: String,
) -> Result<DualRecordingResult, String> {
    // Get app data directory for storing recordings
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;

    let recordings_dir = app_data_dir.join("recordings");
    std::fs::create_dir_all(&recordings_dir).map_err(|e| e.to_string())?;

    // Mic recording path
    let mic_filename = format!("{}_mic.wav", note_id);
    let mic_path = recordings_dir.join(&mic_filename);

    // System audio recording path
    let system_filename = format!("{}_system.wav", note_id);
    let system_path = recordings_dir.join(&system_filename);

    // Start mic recording
    audio::start_recording(state.recording.clone(), mic_path.clone())
        .map_err(|e| e.to_string())?;

    // Try to start system audio recording if available
    let system_started = {
        let capture = state.system_capture.lock().map_err(|e| e.to_string())?;

        if let Some(cap) = capture.as_ref() {
            match cap.start(system_path.clone()) {
                Ok(()) => {
                    // Store the system output path
                    let mut sys_path = state.system_output_path.lock().map_err(|e| e.to_string())?;
                    *sys_path = Some(system_path.clone());
                    true
                }
                Err(e) => {
                    eprintln!("Failed to start system audio capture: {}", e);
                    false
                }
            }
        } else {
            false
        }
    };

    Ok(DualRecordingResult {
        mic_path: Some(mic_path.to_string_lossy().to_string()),
        system_path: if system_started {
            Some(system_path.to_string_lossy().to_string())
        } else {
            None
        },
        playback_path: None, // Will be set when recording stops
    })
}

/// Stop dual recording and merge files for playback
/// Returns the result with all paths including the merged playback file
#[tauri::command]
pub fn stop_dual_recording(
    app: AppHandle,
    state: State<AudioState>,
    db: State<Database>,
    note_id: String,
) -> Result<DualRecordingResult, String> {
    // Stop mic recording
    let mic_path = audio::stop_recording(&state.recording)
        .map_err(|e| e.to_string())?
        .ok_or("No mic recording path found")?;

    // Stop system audio recording
    let system_path = {
        let capture = state.system_capture.lock().map_err(|e| e.to_string())?;

        if let Some(cap) = capture.as_ref() {
            cap.stop().map_err(|e| e.to_string())?
        } else {
            None
        }
    };

    // Clear stored system path
    {
        let mut sys_path = state.system_output_path.lock().map_err(|e| e.to_string())?;
        *sys_path = None;
    }

    // Compacted here too. This path predates segment tracking and has no row to
    // update, but the files it leaves behind are the same size as any other.
    let segment_id = state.recording.current_segment_db_id.load(Ordering::SeqCst);
    let (mic_path, system_path) =
        compact_segment_audio(&db, segment_id, &mic_path, system_path.as_deref());

    // Same whole-note rebuild as the segment-aware stop below. Routing both
    // through one builder is deliberate: this path used to merge only the file
    // it had just closed, which is exactly how playback lost earlier segments.
    let playback_path = build_note_playback(&app, &db, &note_id);

    Ok(DualRecordingResult {
        mic_path: Some(mic_path.to_string_lossy().to_string()),
        system_path: system_path.map(|p| p.to_string_lossy().to_string()),
        playback_path,
    })
}


/// Shrink a just-finished segment's audio and point the database at it.
///
/// Recordings are kept as 16 kHz mono FLAC — see `audio::codec` for why that
/// is the right shape and what it gives up. Conversion happens here, when the
/// segment closes, rather than in the capture callbacks: the real-time path
/// stays exactly as it was, and one piece of code serves both new recordings
/// and compaction of the existing library.
///
/// **A failure here never fails the stop.** The meeting has been recorded; the
/// files are on disk and the database points at them. Losing that because a
/// compressor did not like something would turn a disk-space feature into the
/// worst bug this app could have. Anything that goes wrong leaves the original
/// WAV in place and says so.
fn compact_segment_audio(
    db: &Database,
    segment_id: i64,
    mic: &std::path::Path,
    system: Option<&std::path::Path>,
) -> (std::path::PathBuf, Option<std::path::PathBuf>) {
    let compact_one = |path: &std::path::Path| -> std::path::PathBuf {
        match audio::codec::compact(path) {
            Ok(done) => {
                let saved = done.before_bytes.saturating_sub(done.after_bytes);
                println!(
                    "[audio] compacted {} — {} KB saved",
                    path.display(),
                    saved / 1024
                );
                done.path
            }
            Err(e) => {
                eprintln!(
                    "[audio] could not compact {} ({e}); keeping the original",
                    path.display()
                );
                path.to_path_buf()
            }
        }
    };

    let mic_out = compact_one(mic);
    let system_out = system.map(compact_one);

    // Only recorded once the files are actually in place. A row naming a file
    // that does not exist is worse than a row naming a large one.
    if segment_id > 0 {
        let sys = system_out.as_ref().map(|p| p.to_string_lossy().to_string());
        if let Err(e) = db.update_segment_paths(
            segment_id,
            &mic_out.to_string_lossy(),
            sys.as_deref(),
        ) {
            eprintln!("[audio] compacted the audio but could not record where it went: {e}");
        }
    }

    (mic_out, system_out)
}

/// Stop dual recording with segment tracking - updates segment duration in database
#[tauri::command]
pub fn stop_dual_recording_with_segments(
    app: AppHandle,
    state: State<AudioState>,
    db: State<Database>,
    note_id: String,
) -> Result<DualRecordingResult, String> {
    // Get the recording duration before stopping
    let duration_ms = state.recording.get_segment_elapsed_ms();

    // Stop mic recording
    let mic_path = audio::stop_recording(&state.recording)
        .map_err(|e| e.to_string())?
        .ok_or("No mic recording path found")?;

    // Stop system audio recording
    let system_path = {
        let capture = state.system_capture.lock().map_err(|e| e.to_string())?;

        if let Some(cap) = capture.as_ref() {
            cap.stop().map_err(|e| e.to_string())?
        } else {
            None
        }
    };

    // Clear stored system path
    {
        let mut sys_path = state.system_output_path.lock().map_err(|e| e.to_string())?;
        *sys_path = None;
    }

    // Update segment duration in database
    let segment_id = state.recording.current_segment_db_id.load(Ordering::SeqCst);
    if segment_id > 0 {
        let _ = db.update_segment_duration(segment_id, duration_ms);
    }

    // Shrink what was just recorded, before playback is rebuilt — so the mix
    // reads the files the database now names rather than ones about to be
    // replaced underneath it.
    let (mic_path, system_path) =
        compact_segment_audio(&db, segment_id, &mic_path, system_path.as_deref());

    // Rebuild playback from every segment in the note, not just the one that
    // just stopped — otherwise continuing a recording throws away the audio of
    // everything before it.
    let playback_path = build_note_playback(&app, &db, &note_id);

    Ok(DualRecordingResult {
        mic_path: Some(mic_path.to_string_lossy().to_string()),
        system_path: system_path.map(|p| p.to_string_lossy().to_string()),
        playback_path,
    })
}


/// What a pass over the library recovered.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CompactionReport {
    pub files_examined: usize,
    pub files_compacted: usize,
    pub files_failed: usize,
    pub bytes_before: u64,
    pub bytes_after: u64,
    /// What went wrong, and where. A count alone leaves the only explanation in
    /// a console the user cannot see.
    pub failures: Vec<CompactionFailure>,
    /// Files that were on disk with nothing in the database pointing at them.
    ///
    /// Deleting a note removes its rows and leaves its audio behind, so these
    /// accumulate. They are compacted like anything else — the space is real —
    /// but they are counted separately because nothing will ever play them and
    /// the honest thing is to say so rather than let them look like part of the
    /// library.
    pub orphans: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CompactionFailure {
    pub path: String,
    pub reason: String,
}

/// Rewrite every recording in the library as 16 kHz mono FLAC.
///
/// This is where the space already spent actually comes back — new recordings
/// are compacted as they finish, but nothing touches what is already on disk
/// until this runs.
///
/// Safe to interrupt and safe to re-run: each file is converted, verified
/// readable and only then does the original go, and a file already in the
/// target format is skipped rather than rewritten. A file that fails is
/// counted, left exactly as it was, and does not stop the pass — one unreadable
/// recording should not block recovering the rest.
#[tauri::command]
pub fn compact_recordings(app: AppHandle, db: State<Database>) -> Result<CompactionReport, String> {
    let recordings_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("no app data directory: {e}"))?
        .join("recordings");

    let mut report = CompactionReport {
        files_examined: 0,
        files_compacted: 0,
        files_failed: 0,
        bytes_before: 0,
        bytes_after: 0,
        failures: Vec::new(),
        orphans: 0,
    };

    // Walk the directory rather than the database.
    //
    // Two earlier passes worked from DB rows and each missed most of the disk,
    // because plenty of files have no row: deleting a note removes its rows and
    // leaves the audio, playback mixes exist for notes whose audio_path was
    // cleared, and uploads leave temporaries. The directory is the only honest
    // account of what is taking up space.
    let entries = match std::fs::read_dir(&recordings_dir) {
        Ok(e) => e,
        Err(e) => return Err(format!("could not read {}: {e}", recordings_dir.display())),
    };

    // What moved, so database rows naming the old path can be corrected after.
    let mut moved: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        // Already done, or the debris of an interrupted conversion.
        if ext == "flac" || ext == "partial" {
            continue;
        }
        if !audio::converter::is_supported_format(&path) {
            continue;
        }

        report.files_examined += 1;
        match audio::codec::compact(&path) {
            Ok(done) => {
                report.bytes_before += done.before_bytes;
                report.bytes_after += done.after_bytes;
                if done.path != path {
                    report.files_compacted += 1;
                    moved.insert(
                        path.to_string_lossy().to_string(),
                        done.path.to_string_lossy().to_string(),
                    );
                }
            }
            Err(e) => {
                report.files_failed += 1;
                report.failures.push(CompactionFailure {
                    path: path.display().to_string(),
                    reason: e.to_string(),
                });
                eprintln!("[compact] skipped {} — {e}", path.display());
            }
        }
    }

    // Now correct every database reference to a file that moved. Done after the
    // conversions rather than alongside them, so a row is only ever updated to
    // a path that already exists.
    let mut referenced = std::collections::HashSet::new();

    if let Ok(segments) = db.all_audio_segments() {
        for segment in segments {
            let mic = segment.mic_path.clone();
            let system = segment.system_path.clone();
            for p in [mic.as_ref(), system.as_ref()].into_iter().flatten() {
                referenced.insert(p.clone());
            }
            let new_mic = mic.as_ref().and_then(|p| moved.get(p)).cloned();
            let new_system = system.as_ref().and_then(|p| moved.get(p)).cloned();
            if (new_mic.is_some() || new_system.is_some()) && segment.id > 0 {
                let mic_str = new_mic.or(mic).unwrap_or_default();
                let sys_str = new_system.or(system);
                if let Err(e) = db.update_segment_paths(segment.id, &mic_str, sys_str.as_deref()) {
                    eprintln!("[compact] moved segment {} but could not record it: {e}", segment.id);
                }
            }
        }
    }

    if let Ok(uploads) = db.all_uploaded_audio() {
        for (id, path) in uploads {
            referenced.insert(path.clone());
            if let Some(new_path) = moved.get(&path)
                && let Err(e) = db.update_uploaded_audio_path(id, new_path)
            {
                eprintln!("[compact] moved upload {id} but could not record it: {e}");
            }
        }
    }

    if let Ok(notes) = db.all_note_audio_paths() {
        for (note_id, path) in notes {
            referenced.insert(path.clone());
            if let Some(new_path) = moved.get(&path)
                && let Err(e) = db.update_note_audio_path(&note_id, new_path)
            {
                eprintln!("[compact] moved the playback track for {note_id} but could not record it: {e}");
            }
        }
    }

    report.orphans = moved.keys().filter(|p| !referenced.contains(*p)).count();

    println!(
        "[compact] {} of {} files, {} MB -> {} MB, {} failed, {} orphaned",
        report.files_compacted,
        report.files_examined,
        report.bytes_before / 1_048_576,
        report.bytes_after / 1_048_576,
        report.files_failed,
        report.orphans
    );

    Ok(report)
}

/// Rebuild `{note_id}.wav` from all of the note's recording segments, in order.
///
/// Returns `None` (having logged) if it cannot be built — playback is a
/// convenience, and the per-segment files are still on disk either way, so a
/// failure here must not fail the stop that produced them.
fn build_note_playback(app: &AppHandle, db: &Database, note_id: &str) -> Option<String> {
    let recordings_dir = match app.path().app_data_dir() {
        Ok(dir) => dir.join("recordings"),
        Err(e) => {
            eprintln!("Playback: no app data dir: {}", e);
            return None;
        }
    };

    let segments = match db.get_audio_segments(note_id) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Playback: could not read audio segments: {}", e);
            return None;
        }
    };

    // get_audio_segments already returns display order; keep it.
    let inputs: Vec<(PathBuf, Option<PathBuf>)> = segments
        .iter()
        .filter_map(|seg| {
            seg.mic_path
                .as_ref()
                .map(|mic| (PathBuf::from(mic), seg.system_path.as_ref().map(PathBuf::from)))
        })
        .collect();

    if inputs.is_empty() {
        return None;
    }

    let playback_file = recordings_dir.join(format!("{}.wav", note_id));
    match build_playback_track(&inputs, &playback_file) {
        // Compacted straight away. The mix is a whole note written out again,
        // so leaving it as WAV would quietly put a second full-size copy of
        // every meeting on disk — which is exactly what the first compaction
        // pass missed. A failure here keeps the WAV, which still plays.
        Ok(()) => match audio::codec::compact(&playback_file) {
            Ok(done) => Some(done.path.to_string_lossy().to_string()),
            Err(e) => {
                eprintln!("Playback: kept the uncompressed mix ({e})");
                Some(playback_file.to_string_lossy().to_string())
            }
        },
        Err(e) => {
            eprintln!("Playback: failed to build track from {} segment(s): {}", inputs.len(), e);
            None
        }
    }
}

/// Duration of a WAV in seconds, from its header alone.
fn wav_duration_secs(path: &PathBuf) -> Option<f64> {
    let reader = hound::WavReader::open(path).ok()?;
    let rate = reader.spec().sample_rate;
    if rate == 0 {
        return None;
    }
    Some(reader.duration() as f64 / rate as f64)
}

/// Whether a note's combined playback track is missing audio.
///
/// Playback is only rewritten when a recording stops, so notes recorded before
/// playback learned to span every segment still hold a file containing just the
/// last one. Compares the track against the segments it should cover rather
/// than tracking a schema version — the files are the truth, and this stays
/// correct if a segment is ever removed or re-recorded.
#[tauri::command]
pub fn playback_needs_rebuild(
    app: AppHandle,
    db: State<Database>,
    note_id: String,
) -> Result<bool, String> {
    let segments = db.get_audio_segments(&note_id).map_err(|e| e.to_string())?;

    // A single segment cannot be truncated, and with none there is nothing to
    // rebuild from.
    if segments.len() < 2 {
        return Ok(false);
    }

    let expected: f64 = segments
        .iter()
        .filter_map(|seg| seg.mic_path.as_ref().or(seg.system_path.as_ref()))
        .map(PathBuf::from)
        .filter_map(|p| wav_duration_secs(&p))
        .sum();

    // Source files gone — nothing to rebuild from, so do not offer it.
    if expected <= 0.0 {
        return Ok(false);
    }

    let recordings_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("recordings");
    let current = wav_duration_secs(&recordings_dir.join(format!("{}.wav", note_id)))
        .unwrap_or(0.0);

    // Tolerance for resampling and rounding across segment boundaries.
    Ok(current < expected * 0.95)
}

/// Rebuild a note's combined playback track from its segments.
///
/// Only regenerates the derived {note_id}.wav; transcripts, notes and the
/// per-segment audio are untouched, so it is safe to run repeatedly.
#[tauri::command]
pub fn rebuild_note_playback(
    app: AppHandle,
    db: State<Database>,
    note_id: String,
) -> Result<String, String> {
    build_note_playback(&app, &db, &note_id)
        .ok_or_else(|| "Could not rebuild playback from this note's recordings".to_string())
}

/// Path to a single recording segment's audio, mixed for playback.
///
/// The list of recordings plays each segment on its own. Handing back the raw
/// mic file made a quiet microphone sound like nothing at all, and it dropped
/// the other side of the conversation entirely — a segment should play back the
/// same way it appears in the full track.
///
/// Built on demand and cached beside the source files: only segments the user
/// actually plays cost anything, and the mix is reused afterwards. Falls back to
/// whichever raw file exists if the mix cannot be built, since playing something
/// beats playing nothing.
#[tauri::command]
pub fn get_segment_playback_path(
    app: AppHandle,
    db: State<Database>,
    segment_id: i64,
) -> Result<String, String> {
    let segment = db
        .get_audio_segment(segment_id)
        .map_err(|e| e.to_string())?
        .ok_or("Segment not found")?;

    let mic = segment.mic_path.as_ref().map(PathBuf::from);
    let system = segment.system_path.as_ref().map(PathBuf::from);

    // Listen-only segments have no mic side; there is nothing to mix.
    let Some(mic) = mic.filter(|p| p.exists()) else {
        return system
            .filter(|p| p.exists())
            .map(|p| p.to_string_lossy().to_string())
            .ok_or_else(|| "Segment has no readable audio".to_string());
    };

    let raw_fallback = mic.to_string_lossy().to_string();

    let Some(system) = system.filter(|p| p.exists()) else {
        // Mic only — the raw file already is the mix.
        return Ok(raw_fallback);
    };

    let recordings_dir = match app.path().app_data_dir() {
        Ok(dir) => dir.join("recordings"),
        Err(_) => return Ok(raw_fallback),
    };
    let mixed = recordings_dir.join(format!(
        "{}_seg{}_mix.wav",
        segment.note_id, segment.segment_index
    ));

    // Reuse an existing mix unless a source has changed since (retranscription
    // and continued recordings can rewrite a segment's audio).
    let is_fresh = |mixed: &PathBuf| -> bool {
        let mixed_time = std::fs::metadata(mixed).and_then(|m| m.modified()).ok();
        let newest_source = [&mic, &system]
            .iter()
            .filter_map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
            .max();
        match (mixed_time, newest_source) {
            (Some(m), Some(s)) => m >= s,
            _ => false,
        }
    };

    if mixed.exists() && is_fresh(&mixed) {
        return Ok(mixed.to_string_lossy().to_string());
    }

    match build_playback_track(&[(mic, Some(system))], &mixed) {
        Ok(()) => Ok(mixed.to_string_lossy().to_string()),
        Err(e) => {
            eprintln!("Failed to build segment playback mix: {}", e);
            Ok(raw_fallback)
        }
    }
}

/// Check if dual recording is currently active
#[tauri::command]
pub fn is_dual_recording(state: State<AudioState>) -> bool {
    let mic_recording = state.recording.is_recording.load(Ordering::SeqCst);

    let system_recording = state
        .system_capture
        .lock()
        .ok()
        .and_then(|cap| cap.as_ref().map(|c| c.is_capturing()))
        .unwrap_or(false);

    mic_recording || system_recording
}

// ========== Pause/Resume/Continue Recording Commands ==========

/// Get the current recording phase
#[tauri::command]
pub fn get_recording_phase(state: State<AudioState>) -> u8 {
    state.recording.get_phase() as u8
}

/// Pause the current recording (mic only)
/// Returns the duration of the paused segment in milliseconds
#[tauri::command]
pub fn pause_recording_cmd(state: State<AudioState>) -> Result<i64, String> {
    audio::pause_recording(&state.recording).map_err(|e| e.to_string())
}

/// Resume a paused recording (mic only)
#[tauri::command]
pub fn resume_recording_cmd(
    app: AppHandle,
    state: State<AudioState>,
    note_id: String,
) -> Result<String, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;

    let recordings_dir = app_data_dir.join("recordings");
    std::fs::create_dir_all(&recordings_dir).map_err(|e| e.to_string())?;

    // Get the next segment index
    let segment_index = state.recording.current_segment_index.load(Ordering::SeqCst);

    let filename = format!("{}_seg{}.wav", note_id, segment_index);
    let output_path = recordings_dir.join(&filename);

    audio::resume_recording(state.recording.clone(), output_path.clone())
        .map_err(|e| e.to_string())?;

    Ok(output_path.to_string_lossy().to_string())
}

/// Pause dual recording (mic + system audio)
/// Returns the duration of the paused segment in milliseconds
#[tauri::command]
pub fn pause_dual_recording(
    state: State<AudioState>,
    db: State<Database>,
) -> Result<i64, String> {
    // Pause mic recording first
    let duration_ms = audio::pause_recording(&state.recording).map_err(|e| e.to_string())?;

    // Stop system audio capture
    {
        let capture = state.system_capture.lock().map_err(|e| e.to_string())?;
        if let Some(cap) = capture.as_ref() {
            let _ = cap.stop();
        }
    }

    // Update the segment duration in the database
    let segment_id = state.recording.current_segment_db_id.load(Ordering::SeqCst);
    if segment_id > 0 {
        let _ = db.update_segment_duration(segment_id, duration_ms);
    }

    Ok(duration_ms)
}

/// Resume dual recording after pause
/// Returns paths to the new segment files
#[tauri::command]
pub fn resume_dual_recording(
    app: AppHandle,
    state: State<AudioState>,
    db: State<Database>,
    note_id: String,
) -> Result<DualRecordingResult, String> {
    let current_phase = state.recording.get_phase();
    if current_phase != RecordingPhase::Paused {
        return Err("Recording is not paused".to_string());
    }

    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;

    let recordings_dir = app_data_dir.join("recordings");
    std::fs::create_dir_all(&recordings_dir).map_err(|e| e.to_string())?;

    // Get the next segment index from database
    let segment_index = db
        .get_next_segment_index(&note_id)
        .map_err(|e| e.to_string())?;

    // Calculate start offset from previous segments
    let start_offset_ms = db
        .get_total_segment_duration(&note_id)
        .map_err(|e| e.to_string())?;

    // Update state with new segment info
    state
        .recording
        .current_segment_index
        .store(segment_index as u32, Ordering::SeqCst);
    state
        .recording
        .segment_start_offset_ms
        .store(start_offset_ms, Ordering::SeqCst);

    // Mic recording path with segment index
    let mic_filename = format!("{}_mic_seg{}.wav", note_id, segment_index);
    let mic_path = recordings_dir.join(&mic_filename);

    // System audio recording path with segment index
    let system_filename = format!("{}_system_seg{}.wav", note_id, segment_index);
    let system_path = recordings_dir.join(&system_filename);

    // Add segment to database
    let segment_id = db
        .add_audio_segment(
            &note_id,
            segment_index,
            Some(mic_path.to_string_lossy().as_ref()),
            Some(system_path.to_string_lossy().as_ref()),
            start_offset_ms,
        )
        .map_err(|e| e.to_string())?;

    // Store segment ID for later duration update
    state
        .recording
        .current_segment_db_id
        .store(segment_id, Ordering::SeqCst);

    // Start mic recording
    audio::resume_recording(state.recording.clone(), mic_path.clone())
        .map_err(|e| e.to_string())?;

    // Try to start system audio recording
    let system_started = {
        let capture = state.system_capture.lock().map_err(|e| e.to_string())?;

        if let Some(cap) = capture.as_ref() {
            match cap.start(system_path.clone()) {
                Ok(()) => {
                    let mut sys_path = state.system_output_path.lock().map_err(|e| e.to_string())?;
                    *sys_path = Some(system_path.clone());
                    true
                }
                Err(e) => {
                    eprintln!("Failed to start system audio capture: {}", e);
                    false
                }
            }
        } else {
            false
        }
    };

    Ok(DualRecordingResult {
        mic_path: Some(mic_path.to_string_lossy().to_string()),
        system_path: if system_started {
            Some(system_path.to_string_lossy().to_string())
        } else {
            None
        },
        playback_path: None,
    })
}

/// Continue recording on an ended note
/// Reopens the note and starts a new recording segment
#[tauri::command]
pub fn continue_note_recording(
    app: AppHandle,
    state: State<AudioState>,
    db: State<Database>,
    note_id: String,
) -> Result<DualRecordingResult, String> {
    // First, reopen the note (clear ended_at)
    {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now();

        // Check if note exists
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM notes WHERE id = ?1)",
                [&note_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;

        if !exists {
            return Err("Note not found".to_string());
        }

        // Clear ended_at to reopen the note
        conn.execute(
            "UPDATE notes SET ended_at = NULL, updated_at = ?1 WHERE id = ?2",
            (now.to_rfc3339(), &note_id),
        )
        .map_err(|e| e.to_string())?;
    }

    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;

    let recordings_dir = app_data_dir.join("recordings");
    std::fs::create_dir_all(&recordings_dir).map_err(|e| e.to_string())?;

    // Store note ID in state
    {
        let mut current_note = state
            .recording
            .current_note_id
            .lock()
            .map_err(|e| e.to_string())?;
        *current_note = Some(note_id.clone());
    }

    // Get the next segment index from database
    let segment_index = db
        .get_next_segment_index(&note_id)
        .map_err(|e| e.to_string())?;

    // Calculate start offset from previous segments
    let start_offset_ms = db
        .get_total_segment_duration(&note_id)
        .map_err(|e| e.to_string())?;

    // Update state with segment info
    state
        .recording
        .current_segment_index
        .store(segment_index as u32, Ordering::SeqCst);
    state
        .recording
        .segment_start_offset_ms
        .store(start_offset_ms, Ordering::SeqCst);

    // Mic recording path with segment index
    let mic_filename = format!("{}_mic_seg{}.wav", note_id, segment_index);
    let mic_path = recordings_dir.join(&mic_filename);

    // System audio recording path with segment index
    let system_filename = format!("{}_system_seg{}.wav", note_id, segment_index);
    let system_path = recordings_dir.join(&system_filename);

    // Add segment to database
    let segment_id = db
        .add_audio_segment(
            &note_id,
            segment_index,
            Some(mic_path.to_string_lossy().as_ref()),
            Some(system_path.to_string_lossy().as_ref()),
            start_offset_ms,
        )
        .map_err(|e| e.to_string())?;

    // Store segment ID for later duration update
    state
        .recording
        .current_segment_db_id
        .store(segment_id, Ordering::SeqCst);

    // Start mic recording
    audio::start_recording(state.recording.clone(), mic_path.clone())
        .map_err(|e| e.to_string())?;

    // Try to start system audio recording
    let system_started = {
        let capture = state.system_capture.lock().map_err(|e| e.to_string())?;

        if let Some(cap) = capture.as_ref() {
            match cap.start(system_path.clone()) {
                Ok(()) => {
                    let mut sys_path = state.system_output_path.lock().map_err(|e| e.to_string())?;
                    *sys_path = Some(system_path.clone());
                    true
                }
                Err(e) => {
                    eprintln!("Failed to start system audio capture: {}", e);
                    false
                }
            }
        } else {
            false
        }
    };

    Ok(DualRecordingResult {
        mic_path: Some(mic_path.to_string_lossy().to_string()),
        system_path: if system_started {
            Some(system_path.to_string_lossy().to_string())
        } else {
            None
        },
        playback_path: None,
    })
}

/// Start dual recording with segment tracking
/// This is an enhanced version of start_dual_recording that tracks segments in the database
#[tauri::command]
pub fn start_dual_recording_with_segments(
    app: AppHandle,
    state: State<AudioState>,
    db: State<Database>,
    note_id: String,
) -> Result<DualRecordingResult, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;

    let recordings_dir = app_data_dir.join("recordings");
    std::fs::create_dir_all(&recordings_dir).map_err(|e| e.to_string())?;

    // Reset state for new recording session
    state.recording.reset_for_new_session();

    // Store note ID
    {
        let mut current_note = state
            .recording
            .current_note_id
            .lock()
            .map_err(|e| e.to_string())?;
        *current_note = Some(note_id.clone());
    }

    // Get segment index (should be 0 for new recording)
    let segment_index = db
        .get_next_segment_index(&note_id)
        .map_err(|e| e.to_string())?;

    // Mic recording path with segment index
    let mic_filename = format!("{}_mic_seg{}.wav", note_id, segment_index);
    let mic_path = recordings_dir.join(&mic_filename);

    // System audio recording path with segment index
    let system_filename = format!("{}_system_seg{}.wav", note_id, segment_index);
    let system_path = recordings_dir.join(&system_filename);

    // Add segment to database (start_offset_ms is 0 for first segment)
    let segment_id = db
        .add_audio_segment(
            &note_id,
            segment_index,
            Some(mic_path.to_string_lossy().as_ref()),
            Some(system_path.to_string_lossy().as_ref()),
            0, // First segment starts at 0
        )
        .map_err(|e| e.to_string())?;

    // Store segment ID for later duration update
    state
        .recording
        .current_segment_db_id
        .store(segment_id, Ordering::SeqCst);

    // Start mic recording
    audio::start_recording(state.recording.clone(), mic_path.clone())
        .map_err(|e| e.to_string())?;

    // Try to start system audio recording
    let system_started = {
        let capture = state.system_capture.lock().map_err(|e| e.to_string())?;

        if let Some(cap) = capture.as_ref() {
            match cap.start(system_path.clone()) {
                Ok(()) => {
                    let mut sys_path = state.system_output_path.lock().map_err(|e| e.to_string())?;
                    *sys_path = Some(system_path.clone());
                    true
                }
                Err(e) => {
                    eprintln!("Failed to start system audio capture: {}", e);
                    false
                }
            }
        } else {
            false
        }
    };

    Ok(DualRecordingResult {
        mic_path: Some(mic_path.to_string_lossy().to_string()),
        system_path: if system_started {
            Some(system_path.to_string_lossy().to_string())
        } else {
            None
        },
        playback_path: None,
    })
}

// ========== System-audio-only ("listen-only") recording ==========
// Used when the microphone is unavailable or denied but system audio is supported.
// The user is just listening into a meeting; only system audio is captured.

fn set_phase_for_system_only_session(state: &RecordingState) {
    use std::time::Instant;
    state.set_phase(RecordingPhase::Recording);
    if let Ok(mut start_time) = state.segment_start_time.lock() {
        *start_time = Some(Instant::now());
    }
}

#[tauri::command]
pub fn start_system_only_recording_with_segments(
    app: AppHandle,
    state: State<AudioState>,
    db: State<Database>,
    note_id: String,
) -> Result<DualRecordingResult, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;

    let recordings_dir = app_data_dir.join("recordings");
    std::fs::create_dir_all(&recordings_dir).map_err(|e| e.to_string())?;

    state.recording.reset_for_new_session();

    {
        let mut current_note = state
            .recording
            .current_note_id
            .lock()
            .map_err(|e| e.to_string())?;
        *current_note = Some(note_id.clone());
    }

    let segment_index = db
        .get_next_segment_index(&note_id)
        .map_err(|e| e.to_string())?;

    let system_filename = format!("{}_system_seg{}.wav", note_id, segment_index);
    let system_path = recordings_dir.join(&system_filename);

    let segment_id = db
        .add_audio_segment(
            &note_id,
            segment_index,
            None,
            Some(system_path.to_string_lossy().as_ref()),
            0,
        )
        .map_err(|e| e.to_string())?;

    state
        .recording
        .current_segment_db_id
        .store(segment_id, Ordering::SeqCst);

    // Start system audio capture. Errors here are fatal — without mic or system audio,
    // there's nothing to record.
    {
        let capture = state.system_capture.lock().map_err(|e| e.to_string())?;
        let cap = capture
            .as_ref()
            .ok_or_else(|| "System audio capture not available".to_string())?;
        cap.start(system_path.clone()).map_err(|e| e.to_string())?;
    }
    {
        let mut sys_path = state.system_output_path.lock().map_err(|e| e.to_string())?;
        *sys_path = Some(system_path.clone());
    }

    set_phase_for_system_only_session(&state.recording);

    Ok(DualRecordingResult {
        mic_path: None,
        system_path: Some(system_path.to_string_lossy().to_string()),
        playback_path: None,
    })
}

#[tauri::command]
pub fn stop_system_only_recording_with_segments(
    state: State<AudioState>,
    db: State<Database>,
    _note_id: String,
) -> Result<DualRecordingResult, String> {
    let duration_ms = state.recording.get_segment_elapsed_ms();

    let system_path = {
        let capture = state.system_capture.lock().map_err(|e| e.to_string())?;
        if let Some(cap) = capture.as_ref() {
            cap.stop().map_err(|e| e.to_string())?
        } else {
            None
        }
    };

    {
        let mut sys_path = state.system_output_path.lock().map_err(|e| e.to_string())?;
        *sys_path = None;
    }

    let segment_id = state.recording.current_segment_db_id.load(Ordering::SeqCst);
    if segment_id > 0 {
        let _ = db.update_segment_duration(segment_id, duration_ms);
    }

    state.recording.set_phase(RecordingPhase::Idle);
    state.recording.reset_for_new_session();

    let system_path_str = system_path.as_ref().map(|p| p.to_string_lossy().to_string());

    Ok(DualRecordingResult {
        mic_path: None,
        // Listen-only has only one stream, so playback == system file.
        playback_path: system_path_str.clone(),
        system_path: system_path_str,
    })
}

#[tauri::command]
pub fn pause_system_only_recording(
    state: State<AudioState>,
    db: State<Database>,
) -> Result<i64, String> {
    if state.recording.get_phase() != RecordingPhase::Recording {
        return Err("Recording is not active".to_string());
    }
    let duration_ms = state.recording.get_segment_elapsed_ms();

    {
        let capture = state.system_capture.lock().map_err(|e| e.to_string())?;
        if let Some(cap) = capture.as_ref() {
            let _ = cap.stop();
        }
    }

    state.recording.set_phase(RecordingPhase::Paused);

    let segment_id = state.recording.current_segment_db_id.load(Ordering::SeqCst);
    if segment_id > 0 {
        let _ = db.update_segment_duration(segment_id, duration_ms);
    }

    Ok(duration_ms)
}

#[tauri::command]
pub fn resume_system_only_recording(
    app: AppHandle,
    state: State<AudioState>,
    db: State<Database>,
    note_id: String,
) -> Result<DualRecordingResult, String> {
    if state.recording.get_phase() != RecordingPhase::Paused {
        return Err("Recording is not paused".to_string());
    }

    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;
    let recordings_dir = app_data_dir.join("recordings");
    std::fs::create_dir_all(&recordings_dir).map_err(|e| e.to_string())?;

    let segment_index = db
        .get_next_segment_index(&note_id)
        .map_err(|e| e.to_string())?;
    let start_offset_ms = db
        .get_total_segment_duration(&note_id)
        .map_err(|e| e.to_string())?;

    state
        .recording
        .current_segment_index
        .store(segment_index as u32, Ordering::SeqCst);
    state
        .recording
        .segment_start_offset_ms
        .store(start_offset_ms, Ordering::SeqCst);

    let system_filename = format!("{}_system_seg{}.wav", note_id, segment_index);
    let system_path = recordings_dir.join(&system_filename);

    let segment_id = db
        .add_audio_segment(
            &note_id,
            segment_index,
            None,
            Some(system_path.to_string_lossy().as_ref()),
            start_offset_ms,
        )
        .map_err(|e| e.to_string())?;

    state
        .recording
        .current_segment_db_id
        .store(segment_id, Ordering::SeqCst);

    {
        let capture = state.system_capture.lock().map_err(|e| e.to_string())?;
        let cap = capture
            .as_ref()
            .ok_or_else(|| "System audio capture not available".to_string())?;
        cap.start(system_path.clone()).map_err(|e| e.to_string())?;
    }
    {
        let mut sys_path = state.system_output_path.lock().map_err(|e| e.to_string())?;
        *sys_path = Some(system_path.clone());
    }

    set_phase_for_system_only_session(&state.recording);

    Ok(DualRecordingResult {
        mic_path: None,
        system_path: Some(system_path.to_string_lossy().to_string()),
        playback_path: None,
    })
}
