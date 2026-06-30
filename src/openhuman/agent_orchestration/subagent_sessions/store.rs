use std::fs;
use std::path::{Path, PathBuf};

use super::types::{DurableSubagentSession, SubagentSessionStore};

impl SubagentSessionStore {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }

    pub fn path(&self) -> PathBuf {
        self.workspace_dir
            .join(".openhuman")
            .join("subagent_sessions.json")
    }

    pub fn load(&self) -> Result<Vec<DurableSubagentSession>, String> {
        load_from_path(&self.path())
    }

    pub fn save(&self, sessions: &[DurableSubagentSession]) -> Result<(), String> {
        save_to_path(&self.path(), sessions)
    }
}

pub(crate) fn load_from_path(path: &Path) -> Result<Vec<DurableSubagentSession>, String> {
    match fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw)
            .map_err(|err| format!("failed to parse subagent session store: {err}")),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(format!("failed to read subagent session store: {err}")),
    }
}

pub(crate) fn save_to_path(path: &Path, sessions: &[DurableSubagentSession]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create subagent session dir: {err}"))?;
    }
    let raw = serde_json::to_string_pretty(sessions)
        .map_err(|err| format!("failed to encode subagent session store: {err}"))?;
    let tmp_path = path.with_extension(format!("json.tmp-{}", uuid::Uuid::new_v4().simple()));
    fs::write(&tmp_path, raw)
        .map_err(|err| format!("failed to write temporary subagent session store: {err}"))?;
    fs::rename(&tmp_path, path).map_err(|err| {
        let _ = fs::remove_file(&tmp_path);
        format!("failed to commit subagent session store: {err}")
    })
}
