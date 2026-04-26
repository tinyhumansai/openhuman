use crate::openhuman::curated_memory::types::MemoryFile;
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;

pub struct MemoryStore {
    file_path: PathBuf,
    char_limit: usize,
    write_lock: Mutex<()>,
}

const ENTRY_SEP: &str = "\n§\n";

impl MemoryStore {
    pub fn open(dir: &Path, kind: MemoryFile, char_limit: usize) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let file_path = dir.join(kind.filename());
        if !file_path.exists() {
            // One-time migration: if a legacy root-level MEMORY.md exists in the
            // workspace dir (the parent of `dir`), seed the new store from it so
            // existing workspaces don't lose injected memory on first upgrade.
            let legacy_path = dir.parent().map(|p| p.join(kind.filename()));
            let seed = legacy_path
                .as_deref()
                .filter(|p| p.exists())
                .and_then(|p| std::fs::read_to_string(p).ok())
                .unwrap_or_default();
            // Truncate to char_limit on migration so we never write a file that
            // immediately exceeds the configured cap.
            let seed = if seed.chars().count() > char_limit {
                let mut s = seed;
                let cutoff = s
                    .char_indices()
                    .nth(char_limit)
                    .map(|(i, _)| i)
                    .unwrap_or(s.len());
                s.truncate(cutoff);
                s
            } else {
                seed
            };
            std::fs::write(&file_path, &seed)?;
            if !seed.is_empty() {
                log::debug!(
                    "[curated_memory] migrated legacy {} ({} chars) to {}",
                    kind.filename(),
                    seed.chars().count(),
                    file_path.display(),
                );
            }
        }
        Ok(Self {
            file_path,
            char_limit,
            write_lock: Mutex::new(()),
        })
    }

    pub async fn read(&self) -> std::io::Result<String> {
        tokio::fs::read_to_string(&self.file_path).await
    }

    pub async fn add(&self, entry: &str) -> std::io::Result<()> {
        let _g = self.write_lock.lock().await;
        let current = tokio::fs::read_to_string(&self.file_path).await?;
        let next = if current.is_empty() {
            entry.to_string()
        } else {
            format!("{current}{ENTRY_SEP}{entry}")
        };
        if next.chars().count() > self.char_limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("char limit {} exceeded", self.char_limit),
            ));
        }
        atomic_write(&self.file_path, &next).await
    }

    pub async fn replace(&self, needle: &str, replacement: &str) -> std::io::Result<()> {
        if needle.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "needle must not be empty",
            ));
        }
        let _g = self.write_lock.lock().await;
        let current = tokio::fs::read_to_string(&self.file_path).await?;
        let next = current.replace(needle, replacement);
        if next.chars().count() > self.char_limit {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "char limit"));
        }
        atomic_write(&self.file_path, &next).await
    }

    pub async fn remove(&self, needle: &str) -> std::io::Result<()> {
        if needle.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "needle must not be empty",
            ));
        }
        let _g = self.write_lock.lock().await;
        let current = tokio::fs::read_to_string(&self.file_path).await?;
        // Drop any entry containing `needle`.
        let kept: Vec<&str> = current
            .split(ENTRY_SEP)
            .filter(|e| !e.contains(needle))
            .collect();
        let next = kept.join(ENTRY_SEP);
        atomic_write(&self.file_path, &next).await
    }
}

/// Snapshot both curated-memory files at a single point in time. The returned
/// `MemorySnapshot` is a plain `String` pair — it does NOT hold a reference
/// back to the stores, so subsequent `add` / `replace` / `remove` calls leave
/// the snapshot frozen. Designed to be taken once at session start and reused
/// across turns to preserve LLM prefix-cache hits.
pub async fn snapshot_pair(
    memory_store: &MemoryStore,
    user_store: &MemoryStore,
) -> std::io::Result<crate::openhuman::curated_memory::types::MemorySnapshot> {
    Ok(crate::openhuman::curated_memory::types::MemorySnapshot {
        memory: memory_store.read().await?,
        user: user_store.read().await?,
    })
}

async fn atomic_write(path: &Path, contents: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("md.tmp");
    tokio::fs::write(&tmp, contents).await?;
    tokio::fs::rename(&tmp, path).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn add_replace_remove_read_round_trip_under_char_limit() {
        let tmp = TempDir::new().unwrap();
        let store = MemoryStore::open(tmp.path(), MemoryFile::Memory, 200).unwrap();

        store.add("user prefers terse replies").await.unwrap();
        store.add("project context: openhuman").await.unwrap();
        let s = store.read().await.unwrap();
        assert!(s.contains("terse replies"));
        assert!(s.contains("openhuman"));

        store
            .replace("user prefers terse", "user prefers concise")
            .await
            .unwrap();
        let s = store.read().await.unwrap();
        assert!(s.contains("concise"));
        assert!(!s.contains("terse"));

        store.remove("openhuman").await.unwrap();
        let s = store.read().await.unwrap();
        assert!(!s.contains("openhuman"));
    }

    #[tokio::test]
    async fn snapshot_pair_freezes_at_capture_time() {
        let tmp = TempDir::new().unwrap();
        let mem = MemoryStore::open(tmp.path(), MemoryFile::Memory, 500).unwrap();
        let user = MemoryStore::open(tmp.path(), MemoryFile::User, 500).unwrap();
        mem.add("note one").await.unwrap();
        user.add("user is jwalin").await.unwrap();

        let snap = snapshot_pair(&mem, &user).await.unwrap();
        assert!(snap.memory.contains("note one"));
        assert!(snap.user.contains("jwalin"));

        mem.add("note two").await.unwrap();
        assert!(
            !snap.memory.contains("note two"),
            "snapshot was mutated after a later add() — should be frozen"
        );
        assert!(
            mem.read().await.unwrap().contains("note two"),
            "later add() didn't actually persist"
        );
    }

    #[tokio::test]
    async fn add_rejected_when_char_limit_would_be_exceeded() {
        let tmp = TempDir::new().unwrap();
        let store = MemoryStore::open(tmp.path(), MemoryFile::Memory, 50).unwrap();
        store.add("a".repeat(40).as_str()).await.unwrap();
        let err = store.add("a".repeat(40).as_str()).await.unwrap_err();
        assert!(err.to_string().contains("char limit"), "got {err}");
    }
}
