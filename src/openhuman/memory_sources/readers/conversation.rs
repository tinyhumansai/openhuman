//! Conversation source reader.
//!
//! Treats every agent conversation as a memory source item. When synced,
//! each conversation's messages are stored as durable memory alongside
//! other sources like GitHub, integrations, etc.

use async_trait::async_trait;

use crate::openhuman::config::Config;
use crate::openhuman::memory_sources::types::{
    ContentType, MemorySourceEntry, SourceContent, SourceItem, SourceKind,
};

use super::SourceReader;

pub struct ConversationReader;

#[async_trait]
impl SourceReader for ConversationReader {
    fn kind(&self) -> SourceKind {
        SourceKind::Conversation
    }

    async fn list_items(
        &self,
        _source: &MemorySourceEntry,
        config: &Config,
    ) -> Result<Vec<SourceItem>, String> {
        tracing::debug!("[memory_sources:conversation] list_items");

        let threads_dir = config.workspace_dir.join("threads");
        if !threads_dir.exists() {
            return Ok(Vec::new());
        }

        let mut items = Vec::new();
        let mut entries = tokio::fs::read_dir(&threads_dir)
            .await
            .map_err(|e| format!("failed to read threads dir: {e}"))?;

        loop {
            match entries.next_entry().await {
                Ok(Some(entry)) => {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) != Some("json") {
                        continue;
                    }
                    let id = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default()
                        .to_string();

                    let modified_ms = entry
                        .metadata()
                        .await
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_millis() as i64);

                    items.push(SourceItem {
                        id,
                        title: path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("conversation")
                            .to_string(),
                        updated_at_ms: modified_ms,
                    });
                }
                Ok(None) => break,
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        "[memory_sources:conversation] failed to read directory entry, skipping"
                    );
                    continue;
                }
            }
        }

        tracing::debug!(
            count = items.len(),
            "[memory_sources:conversation] found threads"
        );

        Ok(items)
    }

    async fn read_item(
        &self,
        _source: &MemorySourceEntry,
        item_id: &str,
        config: &Config,
    ) -> Result<SourceContent, String> {
        tracing::debug!(
            item_id = %item_id,
            "[memory_sources:conversation] read_item"
        );

        // Validate item_id to prevent path traversal
        if item_id.contains("..") || item_id.contains('/') || item_id.contains('\\') {
            return Err("invalid item_id: path traversal denied".to_string());
        }

        let threads_dir = config.workspace_dir.join("threads");
        let thread_path = threads_dir.join(format!("{item_id}.json"));

        // Canonicalize and verify containment within threads directory
        if !thread_path.exists() {
            return Err(format!("thread '{item_id}' not found"));
        }
        let canonical_base = std::fs::canonicalize(&threads_dir)
            .map_err(|e| format!("cannot resolve threads dir: {e}"))?;
        let canonical_file = std::fs::canonicalize(&thread_path)
            .map_err(|e| format!("cannot resolve thread path: {e}"))?;
        if !canonical_file.starts_with(&canonical_base) {
            return Err("path traversal denied".to_string());
        }

        let raw = tokio::fs::read_to_string(&canonical_file)
            .await
            .map_err(|e| format!("failed to read thread file: {e}"))?;

        let parsed: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("failed to parse thread JSON: {e}"))?;

        let title = parsed
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or(item_id)
            .to_string();

        let body = format_thread_as_markdown(&parsed);

        Ok(SourceContent {
            id: item_id.to_string(),
            title,
            body,
            content_type: ContentType::Markdown,
            metadata: serde_json::json!({
                "source_type": "conversation",
                "thread_id": item_id,
            }),
        })
    }
}

fn format_thread_as_markdown(thread: &serde_json::Value) -> String {
    let mut out = String::new();

    if let Some(title) = thread.get("title").and_then(|v| v.as_str()) {
        out.push_str(&format!("# {title}\n\n"));
    }

    let messages = thread
        .get("messages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for msg in &messages {
        let role = msg
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");

        if content.is_empty() {
            continue;
        }

        out.push_str(&format!("**{role}**: {content}\n\n"));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn format_thread_produces_markdown() {
        let thread = serde_json::json!({
            "title": "Test chat",
            "messages": [
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi there!"},
            ]
        });
        let md = format_thread_as_markdown(&thread);
        assert!(md.contains("# Test chat"));
        assert!(md.contains("**user**: Hello"));
        assert!(md.contains("**assistant**: Hi there!"));
    }

    #[test]
    fn format_thread_skips_empty_content() {
        let thread = serde_json::json!({
            "title": "Sparse",
            "messages": [
                {"role": "user", "content": ""},
                {"role": "assistant", "content": "Reply"},
                {"role": "user", "content": ""},
            ]
        });
        let md = format_thread_as_markdown(&thread);
        assert!(!md.contains("**user**:"));
        assert!(md.contains("**assistant**: Reply"));
    }

    #[test]
    fn format_thread_handles_missing_title() {
        let thread = serde_json::json!({
            "messages": [{"role": "user", "content": "Hi"}]
        });
        let md = format_thread_as_markdown(&thread);
        assert!(!md.starts_with('#'));
        assert!(md.contains("**user**: Hi"));
    }

    #[test]
    fn format_thread_handles_no_messages() {
        let thread = serde_json::json!({"title": "Empty"});
        let md = format_thread_as_markdown(&thread);
        assert!(md.contains("# Empty"));
        assert_eq!(md.trim(), "# Empty");
    }

    #[tokio::test]
    async fn list_items_returns_empty_when_no_threads_dir() {
        let tmp = tempdir().unwrap();
        let mut config = Config::default();
        config.workspace_dir = tmp.path().to_path_buf();

        let source = conversation_source();
        let reader = ConversationReader;
        let items = reader.list_items(&source, &config).await.unwrap();
        assert!(items.is_empty());
    }

    #[tokio::test]
    async fn list_items_finds_json_thread_files() {
        let tmp = tempdir().unwrap();
        let threads_dir = tmp.path().join("threads");
        fs::create_dir_all(&threads_dir).unwrap();

        fs::write(
            threads_dir.join("thread_abc.json"),
            r#"{"title":"Chat 1","messages":[]}"#,
        )
        .unwrap();
        fs::write(
            threads_dir.join("thread_def.json"),
            r#"{"title":"Chat 2","messages":[]}"#,
        )
        .unwrap();
        // Non-json file should be ignored
        fs::write(threads_dir.join("notes.txt"), "ignored").unwrap();

        let mut config = Config::default();
        config.workspace_dir = tmp.path().to_path_buf();

        let source = conversation_source();
        let reader = ConversationReader;
        let items = reader.list_items(&source, &config).await.unwrap();
        assert_eq!(items.len(), 2);

        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert!(ids.contains(&"thread_abc"));
        assert!(ids.contains(&"thread_def"));
    }

    #[tokio::test]
    async fn read_item_returns_formatted_content() {
        let tmp = tempdir().unwrap();
        let threads_dir = tmp.path().join("threads");
        fs::create_dir_all(&threads_dir).unwrap();

        let thread_json = serde_json::json!({
            "title": "Test Conversation",
            "messages": [
                {"role": "user", "content": "What is 2+2?"},
                {"role": "assistant", "content": "4"},
            ]
        });
        fs::write(
            threads_dir.join("conv_123.json"),
            serde_json::to_string(&thread_json).unwrap(),
        )
        .unwrap();

        let mut config = Config::default();
        config.workspace_dir = tmp.path().to_path_buf();

        let source = conversation_source();
        let reader = ConversationReader;
        let content = reader
            .read_item(&source, "conv_123", &config)
            .await
            .unwrap();

        assert_eq!(content.id, "conv_123");
        assert_eq!(content.title, "Test Conversation");
        assert_eq!(content.content_type, ContentType::Markdown);
        assert!(content.body.contains("**user**: What is 2+2?"));
        assert!(content.body.contains("**assistant**: 4"));
    }

    #[tokio::test]
    async fn read_item_returns_error_for_missing_thread() {
        let tmp = tempdir().unwrap();
        let threads_dir = tmp.path().join("threads");
        fs::create_dir_all(&threads_dir).unwrap();

        let mut config = Config::default();
        config.workspace_dir = tmp.path().to_path_buf();

        let source = conversation_source();
        let reader = ConversationReader;
        let result = reader.read_item(&source, "nonexistent", &config).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[tokio::test]
    async fn read_item_rejects_path_traversal() {
        let tmp = tempdir().unwrap();
        let threads_dir = tmp.path().join("threads");
        fs::create_dir_all(&threads_dir).unwrap();

        let mut config = Config::default();
        config.workspace_dir = tmp.path().to_path_buf();

        let source = conversation_source();
        let reader = ConversationReader;

        let result = reader.read_item(&source, "../config", &config).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("path traversal denied"));

        let result = reader
            .read_item(&source, "foo/../../etc/passwd", &config)
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("path traversal denied"));
    }

    fn conversation_source() -> MemorySourceEntry {
        MemorySourceEntry {
            id: "src_conv".into(),
            kind: SourceKind::Conversation,
            label: "Conversations".into(),
            enabled: true,
            toolkit: None,
            connection_id: None,
            path: None,
            glob: None,
            url: None,
            branch: None,
            paths: Vec::new(),
            query: None,
            since_days: None,
            max_items: None,
            max_commits: None,
            max_issues: None,
            max_prs: None,
            selector: None,
            max_tokens_per_sync: None,
            max_cost_per_sync_usd: None,
            sync_depth_days: None,
        }
    }
}
