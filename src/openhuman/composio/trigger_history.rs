//! Persistent ComposeIO trigger history.
//!
//! Stores every incoming ComposeIO trigger as a JSONL record partitioned by
//! UTC day under `<workspace>/state/triggers/YYYY-MM-DD.jsonl`.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use chrono::Utc;
use fs2::FileExt;

use super::types::{ComposioTriggerHistoryEntry, ComposioTriggerHistoryResult};

static GLOBAL_TRIGGER_HISTORY: OnceLock<Arc<ComposioTriggerHistoryStore>> = OnceLock::new();

const TRIGGER_ARCHIVE_DIR: &str = "triggers";

pub fn init_global(workspace_dir: PathBuf) -> Result<(), String> {
    let expected_archive_dir = workspace_dir.join("state").join(TRIGGER_ARCHIVE_DIR);
    if let Some(existing) = GLOBAL_TRIGGER_HISTORY.get() {
        if existing.archive_dir == expected_archive_dir {
            return Ok(());
        }

        return Err(format!(
            "[composio][history] global store already initialized for {} while attempting {}",
            existing.archive_dir.display(),
            expected_archive_dir.display()
        ));
    }

    let store = Arc::new(ComposioTriggerHistoryStore::new(&workspace_dir)?);
    match GLOBAL_TRIGGER_HISTORY.set(store.clone()) {
        Ok(()) => Ok(()),
        Err(_) => {
            if let Some(existing) = GLOBAL_TRIGGER_HISTORY.get() {
                if existing.archive_dir == store.archive_dir {
                    return Ok(());
                }

                return Err(format!(
                    "[composio][history] global store already initialized for {} while attempting {}",
                    existing.archive_dir.display(),
                    store.archive_dir.display()
                ));
            }

            Err(format!(
                "[composio][history] failed to initialize global store for {}",
                store.archive_dir.display()
            ))
        }
    }
}

pub fn global() -> Option<Arc<ComposioTriggerHistoryStore>> {
    GLOBAL_TRIGGER_HISTORY.get().cloned()
}

pub struct ComposioTriggerHistoryStore {
    archive_dir: PathBuf,
}

impl ComposioTriggerHistoryStore {
    pub fn new(workspace_dir: &Path) -> Result<Self, String> {
        let archive_dir = workspace_dir.join("state").join(TRIGGER_ARCHIVE_DIR);
        fs::create_dir_all(&archive_dir).map_err(|error| {
            format!(
                "[composio][history] failed to create archive directory {}: {error}",
                archive_dir.display()
            )
        })?;

        tracing::debug!(
            archive_dir = %archive_dir.display(),
            "[composio][history] archive initialized"
        );

        Ok(Self { archive_dir })
    }

    pub fn record_trigger(
        &self,
        toolkit: &str,
        trigger: &str,
        metadata_id: &str,
        metadata_uuid: &str,
        payload: &serde_json::Value,
    ) -> Result<ComposioTriggerHistoryEntry, String> {
        let entry = ComposioTriggerHistoryEntry {
            received_at_ms: now_ms(),
            toolkit: toolkit.to_string(),
            trigger: trigger.to_string(),
            metadata_id: metadata_id.to_string(),
            metadata_uuid: metadata_uuid.to_string(),
            payload: payload.clone(),
        };

        let path = self.current_day_file_path();
        let line = serde_json::to_string(&entry)
            .map_err(|error| format!("[composio][history] failed to serialize trigger: {error}"))?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| {
                format!(
                    "[composio][history] failed to open archive file {}: {error}",
                    path.display()
                )
            })?;

        file.lock_exclusive().map_err(|error| {
            format!(
                "[composio][history] failed to lock archive file {}: {error}",
                path.display()
            )
        })?;
        let write_result = writeln!(file, "{line}")
            .and_then(|_| file.flush())
            .map_err(|error| {
                format!(
                    "[composio][history] failed to append archive file {}: {error}",
                    path.display()
                )
            });
        let unlock_result = file.unlock().map_err(|error| {
            format!(
                "[composio][history] failed to unlock archive file {}: {error}",
                path.display()
            )
        });

        write_result?;
        unlock_result?;

        tracing::debug!(
            toolkit = %entry.toolkit,
            trigger = %entry.trigger,
            metadata_id = %entry.metadata_id,
            archive_file = %path.display(),
            "[composio][history] trigger archived"
        );

        Ok(entry)
    }

    pub fn list_recent(&self, limit: usize) -> Result<ComposioTriggerHistoryResult, String> {
        let limit = limit.max(1);
        let mut day_files = self.list_day_files()?;
        day_files.sort_by(|left, right| right.cmp(left));

        let mut entries = Vec::new();
        for file in day_files {
            let mut file_entries = self.read_day_file(&file)?;
            file_entries.reverse();
            for entry in file_entries {
                entries.push(entry);
                if entries.len() >= limit {
                    break;
                }
            }
            if entries.len() >= limit {
                break;
            }
        }

        Ok(ComposioTriggerHistoryResult {
            archive_dir: self.archive_dir.display().to_string(),
            current_day_file: self.current_day_file_path().display().to_string(),
            entries,
        })
    }

    fn list_day_files(&self) -> Result<Vec<PathBuf>, String> {
        let dir = fs::read_dir(&self.archive_dir).map_err(|error| {
            format!(
                "[composio][history] failed to read archive directory {}: {error}",
                self.archive_dir.display()
            )
        })?;

        Ok(dir
            .filter_map(|entry| entry.ok().map(|value| value.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
            .collect())
    }

    fn read_day_file(&self, path: &Path) -> Result<Vec<ComposioTriggerHistoryEntry>, String> {
        let file = OpenOptions::new().read(true).open(path).map_err(|error| {
            format!(
                "[composio][history] failed to open archive file {}: {error}",
                path.display()
            )
        })?;

        let reader = BufReader::new(file);
        let mut entries = Vec::new();

        for line in reader.lines() {
            let line = match line {
                Ok(line) if !line.trim().is_empty() => line,
                Ok(_) => continue,
                Err(error) => {
                    tracing::warn!(
                        archive_file = %path.display(),
                        error = %error,
                        "[composio][history] failed to read line"
                    );
                    continue;
                }
            };

            match serde_json::from_str::<ComposioTriggerHistoryEntry>(&line) {
                Ok(entry) => entries.push(entry),
                Err(error) => {
                    tracing::warn!(
                        archive_file = %path.display(),
                        error = %error,
                        "[composio][history] failed to parse archived trigger line"
                    );
                }
            }
        }

        Ok(entries)
    }

    fn current_day_file_path(&self) -> PathBuf {
        self.archive_dir
            .join(format!("{}.jsonl", Utc::now().format("%Y-%m-%d")))
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archives_triggers_in_daily_jsonl_and_lists_latest_first() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).expect("workspace dir");

        let store = ComposioTriggerHistoryStore::new(&workspace).expect("store");
        store
            .record_trigger(
                "gmail",
                "GMAIL_NEW_GMAIL_MESSAGE",
                "id-1",
                "uuid-1",
                &serde_json::json!({ "subject": "hello" }),
            )
            .expect("record first");
        store
            .record_trigger(
                "notion",
                "NOTION_NEW_PAGE",
                "id-2",
                "uuid-2",
                &serde_json::json!({ "title": "roadmap" }),
            )
            .expect("record second");

        let history = store.list_recent(10).expect("list");
        assert_eq!(history.entries.len(), 2);
        assert_eq!(history.entries[0].metadata_id, "id-2");
        assert_eq!(history.entries[1].metadata_id, "id-1");
        assert!(PathBuf::from(&history.current_day_file).exists());
    }
}
