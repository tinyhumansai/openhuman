//! Session file I/O for AI session management.
//!
//! Handles reading/writing JSONL session transcripts and the session index
//! file. All file operations run in Tauri's async runtime.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::encryption::get_data_dir;

/// Session entry in the index file.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SessionIndexEntry {
    pub session_id: String,
    pub updated_at: i64,
    pub session_file: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub model: String,
    pub compaction_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_flush_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_flush_compaction_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
}

/// Get the sessions directory path (~/.openhuman/sessions/).
fn get_sessions_dir() -> Result<PathBuf, String> {
    let dir = get_data_dir()?.join("sessions");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Create sessions dir: {e}"))?;
    Ok(dir)
}

/// Get the session index file path (~/.openhuman/sessions/sessions.json).
fn get_session_index_path() -> Result<PathBuf, String> {
    Ok(get_sessions_dir()?.join("sessions.json"))
}

/// Get the path for a specific session transcript.
fn get_session_file_path(session_id: &str) -> Result<PathBuf, String> {
    Ok(get_sessions_dir()?.join(format!("{session_id}.jsonl")))
}

// --- Tauri Commands ---

/// Initialize the sessions directory.
#[tauri::command]
pub async fn ai_sessions_init() -> Result<bool, String> {
    get_sessions_dir()?;
    let index_path = get_session_index_path()?;
    if !index_path.exists() {
        std::fs::write(&index_path, "{}").map_err(|e| format!("Create index: {e}"))?;
    }
    Ok(true)
}

/// Load the session index.
#[tauri::command]
pub async fn ai_sessions_load_index() -> Result<serde_json::Value, String> {
    let index_path = get_session_index_path()?;
    if !index_path.exists() {
        return Ok(serde_json::json!({}));
    }
    let content = std::fs::read_to_string(&index_path).map_err(|e| format!("Read index: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("Parse index: {e}"))
}

/// Update a session entry in the index.
#[tauri::command]
pub async fn ai_sessions_update_index(
    session_id: String,
    entry: SessionIndexEntry,
) -> Result<bool, String> {
    let index_path = get_session_index_path()?;

    let content = if index_path.exists() {
        std::fs::read_to_string(&index_path).map_err(|e| format!("Read index: {e}"))?
    } else {
        "{}".to_string()
    };

    let mut index: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&content).map_err(|e| format!("Parse index: {e}"))?;

    let entry_json = serde_json::to_value(&entry).map_err(|e| format!("Serialize entry: {e}"))?;
    index.insert(session_id, entry_json);

    let output =
        serde_json::to_string_pretty(&index).map_err(|e| format!("Serialize index: {e}"))?;
    std::fs::write(&index_path, output).map_err(|e| format!("Write index: {e}"))?;

    Ok(true)
}

/// Append a line to a session transcript (JSONL format).
#[tauri::command]
pub async fn ai_sessions_append_transcript(
    session_id: String,
    line: String,
) -> Result<bool, String> {
    let file_path = get_session_file_path(&session_id)?;

    use std::fs::OpenOptions;
    use std::io::Write;

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file_path)
        .map_err(|e| format!("Open transcript: {e}"))?;

    writeln!(file, "{}", line.trim()).map_err(|e| format!("Write transcript: {e}"))?;

    Ok(true)
}

/// Read a session transcript.
#[tauri::command]
pub async fn ai_sessions_read_transcript(session_id: String) -> Result<Vec<String>, String> {
    let file_path = get_session_file_path(&session_id)?;

    if !file_path.exists() {
        return Ok(Vec::new());
    }

    let content =
        std::fs::read_to_string(&file_path).map_err(|e| format!("Read transcript: {e}"))?;

    Ok(content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(String::from)
        .collect())
}

/// Delete a session (transcript file + index entry).
#[tauri::command]
pub async fn ai_sessions_delete(session_id: String) -> Result<bool, String> {
    // Remove transcript file
    let file_path = get_session_file_path(&session_id)?;
    if file_path.exists() {
        std::fs::remove_file(&file_path).map_err(|e| format!("Delete transcript: {e}"))?;
    }

    // Remove from index
    let index_path = get_session_index_path()?;
    if index_path.exists() {
        let content =
            std::fs::read_to_string(&index_path).map_err(|e| format!("Read index: {e}"))?;
        let mut index: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&content).map_err(|e| format!("Parse index: {e}"))?;
        index.remove(&session_id);
        let output = serde_json::to_string_pretty(&index).map_err(|e| format!("Serialize: {e}"))?;
        std::fs::write(&index_path, output).map_err(|e| format!("Write index: {e}"))?;
    }

    Ok(true)
}

/// List all session IDs.
#[tauri::command]
pub async fn ai_sessions_list() -> Result<Vec<String>, String> {
    let dir = get_sessions_dir()?;
    let mut sessions = Vec::new();

    let entries = std::fs::read_dir(&dir).map_err(|e| format!("Read dir: {e}"))?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".jsonl") {
            sessions.push(name.trim_end_matches(".jsonl").to_string());
        }
    }

    Ok(sessions)
}

/// Read a memory file from ~/.openhuman/.
#[tauri::command]
pub async fn ai_read_memory_file(relative_path: String) -> Result<String, String> {
    let data_dir = get_data_dir()?;
    let file_path = data_dir.join(&relative_path);

    // Security: ensure the path doesn't escape the data directory
    let canonical = file_path
        .canonicalize()
        .map_err(|e| format!("Resolve path: {e}"))?;
    let canonical_data = data_dir
        .canonicalize()
        .map_err(|e| format!("Resolve data dir: {e}"))?;

    if !canonical.starts_with(&canonical_data) {
        return Err("Path traversal denied".to_string());
    }

    std::fs::read_to_string(&canonical).map_err(|e| format!("Read file: {e}"))
}

/// Write a memory file to ~/.openhuman/.
#[tauri::command]
pub async fn ai_write_memory_file(relative_path: String, content: String) -> Result<bool, String> {
    let data_dir = get_data_dir()?;
    let file_path = data_dir.join(&relative_path);

    // Security: ensure the path doesn't escape the data directory
    // For new files, check the parent directory
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Create dirs: {e}"))?;
    }

    // After creating dirs, verify canonical path
    let canonical_data = data_dir
        .canonicalize()
        .map_err(|e| format!("Resolve data dir: {e}"))?;

    // For new files the file itself may not exist yet, so check parent
    let check_path = if file_path.exists() {
        file_path
            .canonicalize()
            .map_err(|e| format!("Resolve: {e}"))?
    } else {
        file_path
            .parent()
            .unwrap()
            .canonicalize()
            .map_err(|e| format!("Resolve parent: {e}"))?
            .join(file_path.file_name().unwrap())
    };

    if !check_path.starts_with(&canonical_data) {
        return Err("Path traversal denied".to_string());
    }

    std::fs::write(&file_path, content).map_err(|e| format!("Write file: {e}"))?;
    Ok(true)
}

/// List memory files in a directory under ~/.openhuman/.
#[tauri::command]
pub async fn ai_list_memory_files(relative_dir: String) -> Result<Vec<String>, String> {
    let data_dir = get_data_dir()?;
    let dir_path = data_dir.join(&relative_dir);

    if !dir_path.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    let entries = std::fs::read_dir(&dir_path).map_err(|e| format!("Read dir: {e}"))?;
    for entry in entries.flatten() {
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            files.push(entry.file_name().to_string_lossy().to_string());
        }
    }

    Ok(files)
}
