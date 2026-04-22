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
            std::fs::write(&file_path, "")?;
        }
        Ok(Self { file_path, char_limit, write_lock: Mutex::new(()) })
    }

    pub async fn read(&self) -> std::io::Result<String> {
        tokio::fs::read_to_string(&self.file_path).await
    }

    pub async fn add(&self, entry: &str) -> std::io::Result<()> {
        let _g = self.write_lock.lock().await;
        let current = tokio::fs::read_to_string(&self.file_path).await.unwrap_or_default();
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
        let _g = self.write_lock.lock().await;
        let current = tokio::fs::read_to_string(&self.file_path).await.unwrap_or_default();
        let next = current.replace(needle, replacement);
        if next.chars().count() > self.char_limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "char limit",
            ));
        }
        atomic_write(&self.file_path, &next).await
    }

    pub async fn remove(&self, needle: &str) -> std::io::Result<()> {
        let _g = self.write_lock.lock().await;
        let current = tokio::fs::read_to_string(&self.file_path).await.unwrap_or_default();
        // Drop any entry containing `needle`.
        let kept: Vec<&str> = current
            .split(ENTRY_SEP)
            .filter(|e| !e.contains(needle))
            .collect();
        let next = kept.join(ENTRY_SEP);
        atomic_write(&self.file_path, &next).await
    }
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

        store.replace("user prefers terse", "user prefers concise").await.unwrap();
        let s = store.read().await.unwrap();
        assert!(s.contains("concise"));
        assert!(!s.contains("terse"));

        store.remove("openhuman").await.unwrap();
        let s = store.read().await.unwrap();
        assert!(!s.contains("openhuman"));
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
