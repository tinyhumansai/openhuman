//! Local folder source reader.
//!
//! Lists files matching a glob pattern under a local directory path
//! and reads their content as markdown or plaintext.

use async_trait::async_trait;
use std::path::{Path, PathBuf};

use crate::openhuman::config::Config;
use crate::openhuman::memory_sources::types::{
    ContentType, MemorySourceEntry, SourceContent, SourceItem, SourceKind,
};

use super::SourceReader;

const DEFAULT_GLOB: &str = "**/*.md";
const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024; // 10 MB

pub struct FolderReader;

#[async_trait]
impl SourceReader for FolderReader {
    fn kind(&self) -> SourceKind {
        SourceKind::Folder
    }

    async fn list_items(
        &self,
        source: &MemorySourceEntry,
        _config: &Config,
    ) -> Result<Vec<SourceItem>, String> {
        let base_path = source
            .path
            .as_deref()
            .ok_or("folder source requires a path")?;
        let pattern = source.glob.as_deref().unwrap_or(DEFAULT_GLOB);

        let base = PathBuf::from(base_path);
        if !base.exists() {
            return Err(format!("folder does not exist: {base_path}"));
        }

        let full_pattern = format!("{}/{pattern}", base_path.trim_end_matches('/'));

        tracing::debug!(
            path = %base_path,
            glob = %pattern,
            "[memory_sources:folder] listing items"
        );

        let entries: Vec<SourceItem> = glob::glob(&full_pattern)
            .map_err(|e| format!("invalid glob pattern: {e}"))?
            .filter_map(|entry| {
                let path = entry.ok()?;
                if !path.is_file() {
                    return None;
                }
                let metadata = std::fs::metadata(&path).ok()?;
                if metadata.len() > MAX_FILE_SIZE {
                    return None;
                }
                let rel = path.strip_prefix(&base).ok()?;
                let title = rel.to_string_lossy().to_string();
                let modified_ms = metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64);

                Some(SourceItem {
                    id: rel.to_string_lossy().to_string(),
                    title,
                    updated_at_ms: modified_ms,
                })
            })
            .collect();

        tracing::debug!(count = entries.len(), "[memory_sources:folder] found items");

        Ok(entries)
    }

    async fn read_item(
        &self,
        source: &MemorySourceEntry,
        item_id: &str,
        _config: &Config,
    ) -> Result<SourceContent, String> {
        let base_path = source
            .path
            .as_deref()
            .ok_or("folder source requires a path")?;

        let file_path = Path::new(base_path).join(item_id);

        if !file_path.exists() {
            return Err(format!("file not found: {}", file_path.display()));
        }

        // Prevent path traversal
        let canonical_base = std::fs::canonicalize(base_path)
            .map_err(|e| format!("cannot resolve base path: {e}"))?;
        let canonical_file = std::fs::canonicalize(&file_path)
            .map_err(|e| format!("cannot resolve file path: {e}"))?;
        if !canonical_file.starts_with(&canonical_base) {
            return Err("path traversal denied".to_string());
        }

        // Apply the same size cap as list_items so a huge file can't blow up
        // the renderer or the chunker.
        let metadata = std::fs::metadata(&canonical_file)
            .map_err(|e| format!("failed to stat {}: {e}", canonical_file.display()))?;
        if metadata.len() > MAX_FILE_SIZE {
            return Err(format!(
                "file exceeds {}-byte limit: {}",
                MAX_FILE_SIZE,
                canonical_file.display()
            ));
        }

        let body = tokio::fs::read_to_string(&canonical_file)
            .await
            .map_err(|e| format!("failed to read {}: {e}", canonical_file.display()))?;

        let content_type = if item_id.ends_with(".md") {
            ContentType::Markdown
        } else if item_id.ends_with(".html") || item_id.ends_with(".htm") {
            ContentType::Html
        } else {
            ContentType::Plaintext
        };

        Ok(SourceContent {
            id: item_id.to_string(),
            title: item_id.to_string(),
            body,
            content_type,
            metadata: serde_json::json!({}),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn folder_source(path: &str) -> MemorySourceEntry {
        MemorySourceEntry {
            id: "src_folder".into(),
            kind: SourceKind::Folder,
            label: "Test folder".into(),
            enabled: true,
            toolkit: None,
            connection_id: None,
            path: Some(path.into()),
            glob: None,
            url: None,
            branch: None,
            paths: Vec::new(),
            query: None,
            since_days: None,
            max_items: None,
            selector: None,
        }
    }

    #[tokio::test]
    async fn list_items_finds_md_files() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("note.md"), "# Hello").unwrap();
        fs::write(tmp.path().join("data.txt"), "ignored").unwrap();

        let source = folder_source(&tmp.path().to_string_lossy());
        let reader = FolderReader;
        let items = reader
            .list_items(&source, &Config::default())
            .await
            .unwrap();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "note.md");
    }

    #[tokio::test]
    async fn read_item_returns_file_content() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("test.md"), "# Test\nBody").unwrap();

        let source = folder_source(&tmp.path().to_string_lossy());
        let reader = FolderReader;
        let content = reader
            .read_item(&source, "test.md", &Config::default())
            .await
            .unwrap();

        assert_eq!(content.body, "# Test\nBody");
        assert_eq!(content.content_type, ContentType::Markdown);
    }

    #[tokio::test]
    async fn read_item_prevents_path_traversal() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("safe.md"), "ok").unwrap();

        let source = folder_source(&tmp.path().to_string_lossy());
        let reader = FolderReader;
        let result = reader
            .read_item(&source, "../../../etc/passwd", &Config::default())
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn list_items_nonexistent_folder_errors() {
        let source = folder_source("/nonexistent/path/xyz");
        let reader = FolderReader;
        let result = reader.list_items(&source, &Config::default()).await;
        assert!(result.is_err());
    }
}
