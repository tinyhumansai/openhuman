use std::fs;
use std::path::{Path, PathBuf};

use log::{debug, trace, warn};

use crate::openhuman::config::Config;

use super::types::{GameplayPresetInput, GameplayReviewPreset, GameplayReviewSession};

const REVIEW_DIR: &str = "gameplay_review";
const SESSIONS_DIR: &str = "sessions";
const PRESETS_DIR: &str = "presets";

pub fn review_root(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join(REVIEW_DIR)
}

pub fn sessions_dir(workspace_dir: &Path) -> PathBuf {
    review_root(workspace_dir).join(SESSIONS_DIR)
}

pub fn presets_dir(workspace_dir: &Path) -> PathBuf {
    review_root(workspace_dir).join(PRESETS_DIR)
}

pub fn slugify(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_was_dash = false;
    for ch in value.trim().to_ascii_lowercase().chars() {
        let next = if ch.is_ascii_alphanumeric() { ch } else { '-' };
        if next == '-' {
            if last_was_dash {
                continue;
            }
            last_was_dash = true;
        } else {
            last_was_dash = false;
        }
        out.push(next);
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn ensure_dirs(workspace_dir: &Path) -> Result<(), String> {
    debug!(
        "[gameplay_review][store] ensure_dirs workspace_dir={}",
        workspace_dir.display()
    );
    fs::create_dir_all(sessions_dir(workspace_dir))
        .map_err(|err| format!("failed to create gameplay review sessions dir: {err}"))?;
    fs::create_dir_all(presets_dir(workspace_dir))
        .map_err(|err| format!("failed to create gameplay review presets dir: {err}"))?;
    Ok(())
}

fn write_json_atomic(path: &Path, entity: &str, payload: &str) -> Result<(), String> {
    let tmp_path = path.with_extension("json.tmp");
    trace!(
        "[gameplay_review][store] write_json_atomic entity={} path={} tmp_path={}",
        entity,
        path.display(),
        tmp_path.display()
    );
    fs::write(&tmp_path, payload).map_err(|err| format!("failed to write {entity} tmp: {err}"))?;
    fs::rename(&tmp_path, path)
        .map_err(|err| format!("failed to move {entity} into place: {err}"))?;
    Ok(())
}

/// Returns an error if `session_id` contains path separators or is empty,
/// preventing path traversal when the id comes from an RPC caller.
fn validate_session_id(session_id: &str) -> Result<(), String> {
    if session_id.is_empty() {
        return Err("session_id must not be empty".to_string());
    }
    if session_id.contains('/') || session_id.contains('\\') || session_id.contains("..") {
        return Err(format!(
            "session_id contains invalid characters: {session_id}"
        ));
    }
    Ok(())
}

pub fn session_path(workspace_dir: &Path, session_id: &str) -> PathBuf {
    sessions_dir(workspace_dir).join(format!("{session_id}.json"))
}

pub fn preset_path(workspace_dir: &Path, game_id: &str) -> PathBuf {
    presets_dir(workspace_dir).join(format!("{}.json", slugify(game_id)))
}

pub fn save_session(workspace_dir: &Path, session: &GameplayReviewSession) -> Result<(), String> {
    validate_session_id(&session.session_id)?;
    ensure_dirs(workspace_dir)?;
    let path = session_path(workspace_dir, &session.session_id);
    debug!(
        "[gameplay_review][store] save_session session_id={} game_id={} path={}",
        session.session_id,
        session.game_id,
        path.display()
    );
    let payload = serde_json::to_string_pretty(session)
        .map_err(|err| format!("failed to serialize session: {err}"))?;
    write_json_atomic(&path, "session", &payload)?;
    Ok(())
}

pub fn load_session(
    workspace_dir: &Path,
    session_id: &str,
) -> Result<Option<GameplayReviewSession>, String> {
    validate_session_id(session_id)?;
    let path = session_path(workspace_dir, session_id);
    trace!(
        "[gameplay_review][store] load_session session_id={} path={}",
        session_id,
        path.display()
    );
    if !path.exists() {
        debug!(
            "[gameplay_review][store] load_session missing session_id={} path={}",
            session_id,
            path.display()
        );
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).map_err(|err| format!("failed to read session: {err}"))?;
    let session = serde_json::from_str(&raw)
        .map_err(|err| format!("failed to parse session {}: {err}", path.display()))?;
    debug!(
        "[gameplay_review][store] load_session loaded session_id={} path={}",
        session_id,
        path.display()
    );
    Ok(Some(session))
}

pub fn list_sessions(workspace_dir: &Path) -> Result<Vec<GameplayReviewSession>, String> {
    let dir = sessions_dir(workspace_dir);
    trace!(
        "[gameplay_review][store] list_sessions dir={}",
        dir.display()
    );
    if !dir.exists() {
        debug!(
            "[gameplay_review][store] list_sessions empty dir={}",
            dir.display()
        );
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|err| format!("failed to list sessions: {err}"))? {
        let entry = entry.map_err(|err| format!("failed to read session entry: {err}"))?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let raw = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read session {}: {err}", path.display()))?;
        match serde_json::from_str::<GameplayReviewSession>(&raw) {
            Ok(session) => sessions.push(session),
            Err(err) => warn!(
                "[gameplay_review][store] list_sessions skipped invalid session path={} error={}",
                path.display(),
                err
            ),
        }
    }

    sessions.sort_by(|left, right| {
        right
            .imported_at_ms
            .cmp(&left.imported_at_ms)
            .then(right.analyzed_at_ms.cmp(&left.analyzed_at_ms))
    });
    debug!(
        "[gameplay_review][store] list_sessions complete dir={} count={}",
        dir.display(),
        sessions.len()
    );
    Ok(sessions)
}

pub fn save_preset(workspace_dir: &Path, preset: &GameplayReviewPreset) -> Result<(), String> {
    ensure_dirs(workspace_dir)?;
    let path = preset_path(workspace_dir, &preset.game_id);
    debug!(
        "[gameplay_review][store] save_preset game_id={} path={}",
        preset.game_id,
        path.display()
    );
    let payload = serde_json::to_string_pretty(preset)
        .map_err(|err| format!("failed to serialize preset: {err}"))?;
    write_json_atomic(&path, "preset", &payload)?;
    Ok(())
}

pub fn load_preset(
    workspace_dir: &Path,
    game_id: &str,
) -> Result<Option<GameplayReviewPreset>, String> {
    let path = preset_path(workspace_dir, game_id);
    trace!(
        "[gameplay_review][store] load_preset game_id={} path={}",
        game_id,
        path.display()
    );
    if !path.exists() {
        debug!(
            "[gameplay_review][store] load_preset missing game_id={} path={}",
            game_id,
            path.display()
        );
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).map_err(|err| format!("failed to read preset: {err}"))?;
    let preset = serde_json::from_str(&raw)
        .map_err(|err| format!("failed to parse preset {}: {err}", path.display()))?;
    debug!(
        "[gameplay_review][store] load_preset loaded game_id={} path={}",
        game_id,
        path.display()
    );
    Ok(Some(preset))
}

pub fn list_presets(workspace_dir: &Path) -> Result<Vec<GameplayReviewPreset>, String> {
    let dir = presets_dir(workspace_dir);
    trace!(
        "[gameplay_review][store] list_presets dir={}",
        dir.display()
    );
    if !dir.exists() {
        debug!(
            "[gameplay_review][store] list_presets empty dir={}",
            dir.display()
        );
        return Ok(Vec::new());
    }

    let mut presets = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|err| format!("failed to list presets: {err}"))? {
        let entry = entry.map_err(|err| format!("failed to read preset entry: {err}"))?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let raw = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read preset {}: {err}", path.display()))?;
        match serde_json::from_str::<GameplayReviewPreset>(&raw) {
            Ok(preset) => presets.push(preset),
            Err(err) => warn!(
                "[gameplay_review][store] list_presets skipped invalid preset path={} error={}",
                path.display(),
                err
            ),
        }
    }

    presets.sort_by(|left, right| left.display_name.cmp(&right.display_name));
    debug!(
        "[gameplay_review][store] list_presets complete dir={} count={}",
        dir.display(),
        presets.len()
    );
    Ok(presets)
}

pub fn workspace_dir_from_config(config: &Config) -> PathBuf {
    config.workspace_dir.clone()
}

pub fn preset_from_input(input: GameplayPresetInput) -> GameplayReviewPreset {
    GameplayReviewPreset {
        game_id: input.game_id,
        display_name: input.display_name,
        coaching_focus: input.coaching_focus,
        audio_feedback: input.audio_feedback,
        spoiler_mode: input.spoiler_mode,
        notes: input.notes,
        updated_at_ms: chrono::Utc::now().timestamp_millis(),
    }
}
