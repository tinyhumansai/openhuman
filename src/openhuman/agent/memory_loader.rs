use std::path::PathBuf;

use crate::openhuman::memory::Memory;
use crate::openhuman::util::provenance_tag;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::harness::memory_context::{
    CROSS_CHAT_LIMIT, CROSS_CHAT_SNIPPET_CHARS, WORKING_MEMORY_KEY_PREFIX, WORKING_MEMORY_LIMIT,
};
use crate::openhuman::learning::transcript_ingest::CONVERSATION_MEMORY_NAMESPACE;
use crate::openhuman::memory_conversations::ConversationStore;

/// Maximum number of `[Prior conversations]` lines surfaced into the prompt
/// at the start of a fresh chat. Tight cap on purpose: this block is meant
/// to recover continuity for high-importance facts, not to dump session
/// history into context. See issue #1399.
const PRIOR_CONVERSATION_LIMIT: usize = 3;
/// Only the importance prefix `high.` survives into the prompt block.
/// Medium/low entries stay queryable via the on-demand memory tool but
/// do not auto-pollute every fresh chat.
const PRIOR_CONVERSATION_KEY_PREFIX: &str = "high.";

/// Parse a `MemoryEntry::timestamp` (RFC 3339) into an absolute
/// `YYYY-MM-DD` label for prompt injection, e.g. `2026-05-25`. Returns
/// `None` when the timestamp is missing or unparseable so callers omit
/// the stamp rather than emit a garbage date.
///
/// Time-sensitive memory ("finish the proposal by Wednesday") is a prime
/// vector for stale-as-current hallucinations: with no date the model
/// can't tell a four-day-old working fact from a present-tense one, so it
/// may serve it as today's — the same failure as the memory-tree path.
/// This block feeds the chat user message *and*, via
/// `last_memory_context`, every typed sub-agent including the cron
/// morning briefing (#2944). Reuses the prompt layer's absolute-date
/// formatter for one consistent date shape across surfaces.
fn memory_entry_date_label(timestamp: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|dt| {
            crate::openhuman::agent::prompts::memory_date_label(dt.with_timezone(&chrono::Utc))
        })
}

/// Canonical header for the `[Cross-chat context]` block injected on
/// every turn that has FTS-surfaced hits from other threads.
///
/// The "historical" / "capabilities may have changed since" suffix is
/// deliberate: it tells the model these snippets are snapshots from
/// earlier moments and that capability claims (e.g. "I can't delete
/// emails") may be stale because the tool surface or per-toolkit scope
/// toggles can change between chats.
///
/// Single source of truth — all three call sites bind to this constant
/// so a wording tweak doesn't drift between (a) `memory_loader.rs`'s
/// primary JSONL path, (b) `harness/memory_context.rs`'s fallback
/// recall path, and (c) the orchestrator's "Capability questions"
/// prompt section that names the header verbatim. Tests assert on this
/// constant too — see `memory_loader::tests` and
/// `harness::memory_context::tests`.
pub const CROSS_CHAT_HEADER: &str =
    "[Cross-chat context — historical; capabilities may have changed since]\n";

#[async_trait]
pub trait MemoryLoader: Send + Sync {
    async fn load_context(&self, memory: &dyn Memory, user_message: &str)
        -> anyhow::Result<String>;
}

pub struct DefaultMemoryLoader {
    limit: usize,
    min_relevance_score: f64,
    /// Maximum characters of memory context to inject (0 = unlimited).
    max_context_chars: usize,
    /// Workspace dir for direct cross-thread JSONL search (issue #1505).
    /// `None` falls back to the Memory-trait recall path.
    workspace_dir: Option<PathBuf>,
}

/// Lightweight citation object derived from recalled memory entries.
///
/// These citations are attached to agent responses so the UI can show
/// provenance for memory-informed answers without exposing full raw memory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryCitation {
    pub id: String,
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    pub timestamp: String,
    pub snippet: String,
}

impl Default for DefaultMemoryLoader {
    fn default() -> Self {
        Self {
            limit: 5,
            min_relevance_score: 0.4,
            max_context_chars: 2000,
            workspace_dir: None,
        }
    }
}

impl DefaultMemoryLoader {
    pub fn new(limit: usize, min_relevance_score: f64) -> Self {
        Self {
            limit: limit.max(1),
            min_relevance_score,
            max_context_chars: 2000,
            workspace_dir: None,
        }
    }

    pub fn with_max_chars(mut self, max_chars: usize) -> Self {
        self.max_context_chars = max_chars;
        self
    }

    /// Wire the workspace dir so the `[Cross-chat context]` block can do
    /// direct JSONL scans across threads (issue #1505). Without this the
    /// loader still falls back to the Memory-trait recall path, which only
    /// surfaces hits from archived chats (episodic_log).
    pub fn with_workspace_dir(mut self, workspace_dir: PathBuf) -> Self {
        self.workspace_dir = Some(workspace_dir);
        self
    }
}

/// Collect citation metadata from semantic memory recall for a user turn.
///
/// This mirrors the primary recall path used by `DefaultMemoryLoader` so the
/// UI can display trusted sources whenever memory context influenced a reply.
pub async fn collect_recall_citations(
    memory: &dyn Memory,
    user_message: &str,
    limit: usize,
    min_relevance_score: f64,
) -> anyhow::Result<Vec<MemoryCitation>> {
    let entries = memory
        .recall(
            user_message,
            limit.max(1),
            crate::openhuman::memory::RecallOpts::default(),
        )
        .await?;

    let citations = entries
        .into_iter()
        .filter(|entry| match entry.score {
            Some(score) => score >= min_relevance_score,
            None => true,
        })
        .map(|entry| {
            let snippet = if entry.content.chars().count() > 280 {
                crate::openhuman::util::truncate_with_ellipsis(&entry.content, 280)
            } else {
                entry.content
            };
            MemoryCitation {
                id: entry.id,
                key: entry.key,
                namespace: entry.namespace,
                score: entry.score,
                timestamp: entry.timestamp,
                snippet,
            }
        })
        .collect();

    Ok(citations)
}

#[async_trait]
impl MemoryLoader for DefaultMemoryLoader {
    async fn load_context(
        &self,
        memory: &dyn Memory,
        user_message: &str,
    ) -> anyhow::Result<String> {
        // Primary `[Memory context]` semantic recall used to be injected here,
        // but it duplicated content the agent can already reach via the
        // compressed memory tree (eager prefetch) and the on-demand memory
        // search tool — and worse, the auto-saved `user_msg` entry would come
        // back as the top "relevant" memory and echo the user's text back at
        // them. Only the bounded `[User working memory]` block remains: it
        // surfaces sync-derived profile facts (timezone, preferences) that the
        // tree digest doesn't always carry, and it is keyed by a fixed
        // `working.user.*` namespace so it can't catch arbitrary chat content.
        let mut context = String::new();
        let budget = if self.max_context_chars > 0 {
            self.max_context_chars
        } else {
            usize::MAX
        };

        let working_query = format!("working.user {user_message}");
        let working_entries = memory
            .recall(
                &working_query,
                WORKING_MEMORY_LIMIT + 2,
                crate::openhuman::memory::RecallOpts::default(),
            )
            .await
            .unwrap_or_default();
        let mut appended_working_header = false;
        for entry in working_entries
            .into_iter()
            .filter(|entry| entry.key.starts_with(WORKING_MEMORY_KEY_PREFIX))
            .filter(|entry| match entry.score {
                Some(score) => score >= self.min_relevance_score,
                None => true,
            })
            .take(WORKING_MEMORY_LIMIT)
        {
            if !appended_working_header {
                let section = "[User working memory]\n";
                if section.len() > budget {
                    break;
                }
                context.push_str(section);
                appended_working_header = true;
            }
            // Stamp each fact with its last-updated date so the model can
            // compare against the current date and not present a stale
            // working fact as current (#2944).
            let line = match memory_entry_date_label(&entry.timestamp) {
                Some(date) => format!("- {} (as of {date}): {}\n", entry.key, entry.content),
                None => format!("- {}: {}\n", entry.key, entry.content),
            };
            if context.len() + line.len() > budget {
                tracing::debug!(
                    budget,
                    current_len = context.len(),
                    skipped_line_len = line.len(),
                    "[memory_loader] context budget reached while appending working memory"
                );
                break;
            }
            context.push_str(&line);
        }

        // ── Prior conversations (issue #1399) ─────────────────────────
        // High-importance, transcript-derived facts from earlier chats.
        // Namespace-scoped recall keeps this block small and tightly
        // bounded — only entries the heuristic extractor flagged as
        // `high.*` are eligible, and only the first short snippet of
        // each is included so the block never crowds out the user's
        // actual message.
        let prior_query = format!("{} {}", CONVERSATION_MEMORY_NAMESPACE, user_message);
        let prior_entries = memory
            .recall(
                &prior_query,
                PRIOR_CONVERSATION_LIMIT * 4,
                crate::openhuman::memory::RecallOpts {
                    namespace: Some(CONVERSATION_MEMORY_NAMESPACE),
                    ..Default::default()
                },
            )
            .await
            .unwrap_or_default();

        let mut appended_prior_header = false;
        let mut prior_added = 0usize;
        for entry in prior_entries
            .into_iter()
            .filter(|e| e.key.starts_with(PRIOR_CONVERSATION_KEY_PREFIX))
            .filter(|e| match e.score {
                Some(score) => score >= self.min_relevance_score,
                None => true,
            })
        {
            if prior_added >= PRIOR_CONVERSATION_LIMIT {
                break;
            }
            // The stored content is two lines:
            //   [high preference] I prefer Postgres ...
            //   [provenance] {"thread_id":"thr_…", ...}
            // For the prompt we keep only the first line so the block
            // stays compact. Provenance survives in the underlying
            // memory entry and is queryable through the memory tool.
            let primary = entry
                .content
                .lines()
                .find(|l| !l.trim_start().starts_with("[provenance]"))
                .unwrap_or(&entry.content)
                .trim();
            if primary.is_empty() {
                continue;
            }
            if !appended_prior_header {
                let section = "[Prior conversations]\n";
                if context.len() + section.len() > budget {
                    break;
                }
                context.push_str(section);
                appended_prior_header = true;
            }
            // Date-stamp the fact so a months-old "high importance"
            // statement isn't read as a present-tense commitment (#2944).
            let line = match memory_entry_date_label(&entry.timestamp) {
                Some(date) => format!("- (noted {date}) {primary}\n"),
                None => format!("- {primary}\n"),
            };
            if context.len() + line.len() > budget {
                tracing::debug!(
                    budget,
                    current_len = context.len(),
                    skipped_line_len = line.len(),
                    "[memory_loader] context budget reached while appending prior conversations"
                );
                break;
            }
            context.push_str(&line);
            prior_added += 1;
        }

        // ── Cross-chat context (#1505) ───────────────────────────────────
        //
        // Same user, multiple chats. Primary source: direct JSONL scan
        // across `<workspace>/memory/conversations/threads/*.jsonl` via
        // `ConversationStore::search_cross_thread_messages`. JSONL is
        // append-per-turn, so cross-chat hits surface immediately —
        // unlike the durable-fact pipeline (`learning::transcript_ingest`)
        // which is async/batched and the episodic_log archivist path
        // which only fires on explicit `archive_session`.
        //
        // The current chat's `thread_id` (from the channel-side
        // `with_thread_id` task-local) is excluded so the block doesn't
        // echo same-chat history.
        //
        // Fallback: when `workspace_dir` is not wired (e.g. tests, or a
        // headless run that didn't go through the session builder), call
        // through `memory.recall` with `cross_session=true` instead.
        // That path reads `episodic_log` (populated only by the
        // archivist tool) so it's a best-effort secondary signal.
        let current_thread_id =
            crate::openhuman::inference::provider::thread_context::current_thread_id();
        let cross_hits: Vec<(String, String)> = if let Some(workspace_dir) = &self.workspace_dir {
            let store = ConversationStore::new(workspace_dir.clone());
            match store.search_cross_thread_messages(
                user_message,
                CROSS_CHAT_LIMIT * 4,
                current_thread_id.as_deref(),
            ) {
                Ok(hits) => {
                    tracing::debug!(
                        "[memory_loader] cross-chat JSONL scan returned {} hits (exclude={:?})",
                        hits.len(),
                        current_thread_id
                    );
                    hits.into_iter()
                        .filter(|h| h.score >= self.min_relevance_score)
                        .take(CROSS_CHAT_LIMIT)
                        .map(|h| (h.thread_id, h.content))
                        .collect()
                }
                Err(e) => {
                    tracing::warn!("[memory_loader] cross-chat JSONL scan failed (non-fatal): {e}");
                    Vec::new()
                }
            }
        } else {
            // Fallback path (no workspace_dir wired)
            let cross_session_opts = crate::openhuman::memory::RecallOpts {
                session_id: current_thread_id.as_deref(),
                cross_session: true,
                min_score: Some(self.min_relevance_score),
                ..Default::default()
            };
            let entries = memory
                .recall(user_message, CROSS_CHAT_LIMIT * 3, cross_session_opts)
                .await
                .unwrap_or_default();
            entries
                .into_iter()
                .filter(|e| e.id.starts_with("episodic-cross:"))
                .filter(|e| {
                    // Fallback entries may carry a JSON session blob
                    // (`{"thread_id": "...", "client_id": "..."}`) rather
                    // than a bare thread_id, so the SQL-side exclusion
                    // can miss. Re-check on this side using the same
                    // normalization shape.
                    let Some(current_tid) = current_thread_id.as_deref() else {
                        return true;
                    };
                    let Some(raw_sid) = e.session_id.as_deref() else {
                        return true;
                    };
                    let sid_thread = serde_json::from_str::<serde_json::Value>(raw_sid)
                        .ok()
                        .and_then(|v| {
                            v.get("thread_id")
                                .and_then(|t| t.as_str().map(|s| s.to_string()))
                        })
                        .unwrap_or_else(|| raw_sid.to_string());
                    sid_thread != current_tid
                })
                .filter(|e| match e.score {
                    Some(score) => score >= self.min_relevance_score,
                    None => true,
                })
                .take(CROSS_CHAT_LIMIT)
                .map(|e| {
                    let sid = e
                        .session_id
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string());
                    (sid, e.content)
                })
                .collect()
        };

        let mut appended_cross_header = false;
        for (sid, content) in cross_hits {
            let snippet = if content.chars().count() > CROSS_CHAT_SNIPPET_CHARS {
                crate::openhuman::util::truncate_with_ellipsis(&content, CROSS_CHAT_SNIPPET_CHARS)
            } else {
                content
            };
            let prov = provenance_tag(&sid);
            if !appended_cross_header {
                // The header explicitly labels these snippets as historical so
                // the model down-weights them when answering capability
                // questions — see CROSS_CHAT_HEADER doc for the rationale and
                // the cross-module wording contract.
                if context.len() + CROSS_CHAT_HEADER.len() > budget {
                    break;
                }
                context.push_str(CROSS_CHAT_HEADER);
                appended_cross_header = true;
            }
            let line = format!("- [{prov}] {snippet}\n");
            if context.len() + line.len() > budget {
                tracing::debug!(
                    budget,
                    current_len = context.len(),
                    skipped_line_len = line.len(),
                    "[memory_loader] context budget reached while appending cross-chat context"
                );
                break;
            }
            context.push_str(&line);
        }

        if context.is_empty() {
            return Ok(String::new());
        }
        context.push('\n');
        Ok(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry};

    struct MockMemory {
        entries: Vec<MemoryEntry>,
        cross_chat: Vec<MemoryEntry>,
    }

    impl MockMemory {
        fn new(entries: Vec<MemoryEntry>) -> Self {
            Self {
                entries,
                cross_chat: Vec::new(),
            }
        }
    }

    #[async_trait]
    impl Memory for MockMemory {
        fn name(&self) -> &str {
            "mock"
        }

        async fn store(
            &self,
            _namespace: &str,
            _key: &str,
            _content: &str,
            _category: MemoryCategory,
            _session_id: Option<&str>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn recall(
            &self,
            _query: &str,
            _limit: usize,
            opts: crate::openhuman::memory::RecallOpts<'_>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            if opts.cross_session {
                return Ok(self.cross_chat.clone());
            }
            Ok(self.entries.clone())
        }

        async fn get(&self, _namespace: &str, _key: &str) -> anyhow::Result<Option<MemoryEntry>> {
            Ok(None)
        }

        async fn list(
            &self,
            _namespace: Option<&str>,
            _category: Option<&MemoryCategory>,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            Ok(Vec::new())
        }

        async fn forget(&self, _namespace: &str, _key: &str) -> anyhow::Result<bool> {
            Ok(false)
        }

        async fn namespace_summaries(
            &self,
        ) -> anyhow::Result<Vec<crate::openhuman::memory::NamespaceSummary>> {
            Ok(Vec::new())
        }

        async fn count(&self) -> anyhow::Result<usize> {
            Ok(self.entries.len())
        }

        async fn health_check(&self) -> bool {
            true
        }
    }

    fn entry(key: &str, content: &str, score: Option<f64>) -> MemoryEntry {
        MemoryEntry {
            id: format!("id-{key}"),
            key: key.to_string(),
            content: content.to_string(),
            namespace: Some("test".to_string()),
            category: MemoryCategory::Conversation,
            timestamp: "2026-04-22T00:00:00Z".to_string(),
            session_id: None,
            score,
        }
    }

    #[test]
    fn memory_entry_date_label_parses_rfc3339_else_none() {
        assert_eq!(
            super::memory_entry_date_label("2026-05-25T07:00:00Z").as_deref(),
            Some("2026-05-25")
        );
        assert_eq!(super::memory_entry_date_label("not-a-date"), None);
        assert_eq!(super::memory_entry_date_label(""), None);
    }

    #[tokio::test]
    async fn loader_stamps_working_memory_with_date() {
        // #2944: working-memory facts must carry their last-updated date so
        // the model (and downstream sub-agents / the cron briefing, which
        // inherit this block) can tell a stale fact from a current one.
        let mem = MockMemory::new(vec![MemoryEntry {
            id: "id-tz".into(),
            key: "working.user.commitment".into(),
            content: "Finish the proposal by Wednesday.".into(),
            namespace: Some("test".into()),
            category: MemoryCategory::Conversation,
            timestamp: "2026-05-25T00:00:00Z".into(),
            session_id: None,
            score: None,
        }]);

        let out = DefaultMemoryLoader::default()
            .load_context(&mem, "what's on my plate?")
            .await
            .expect("loader must succeed");

        assert!(
            out.contains("[User working memory]"),
            "expected working-memory block, got:\n{out}"
        );
        assert!(
            out.contains("(as of 2026-05-25)"),
            "working-memory fact must carry its date (#2944), got:\n{out}"
        );
    }

    #[tokio::test]
    async fn loader_stamps_prior_conversation_with_date() {
        // #2944: high-importance prior-chat facts must be dated so a
        // months-old statement isn't read as a present-tense commitment.
        let mem = MockMemory::new(vec![MemoryEntry {
            id: "id-1".into(),
            key: "high.preference.aaaaaaaaaaaa".into(),
            content: "[high preference] I prefer Postgres for new services.".into(),
            namespace: Some(super::CONVERSATION_MEMORY_NAMESPACE.to_string()),
            category: MemoryCategory::Conversation,
            timestamp: "2026-04-22T00:00:00Z".into(),
            session_id: Some("thr_old".into()),
            score: Some(0.9),
        }]);

        let out = DefaultMemoryLoader::default()
            .load_context(&mem, "what should I default to for storage?")
            .await
            .expect("loader must succeed");

        assert!(
            out.contains("[Prior conversations]"),
            "expected prior conversations block, got:\n{out}"
        );
        assert!(
            out.contains("(noted 2026-04-22)"),
            "prior-conversation fact must carry its date (#2944), got:\n{out}"
        );
    }

    #[tokio::test]
    async fn loader_surfaces_prior_conversation_high_importance_only() {
        // Prior chat extracted two memories: one high-importance preference
        // and one medium-importance unresolved task. Only the high one
        // should make it into the loader's prompt block (#1399).
        let mem = MockMemory::new(vec![
                MemoryEntry {
                    id: "id-1".into(),
                    key: "high.preference.aaaaaaaaaaaa".into(),
                    content: "[high preference] I prefer Postgres for new services.\n[provenance] {\"thread_id\":\"thr_old\"}".into(),
                    namespace: Some(super::CONVERSATION_MEMORY_NAMESPACE.to_string()),
                    category: MemoryCategory::Conversation,
                    timestamp: "2026-04-22T00:00:00Z".into(),
                    session_id: Some("thr_old".into()),
                    score: Some(0.9),
                },
                MemoryEntry {
                    id: "id-2".into(),
                    key: "med.unresolved_task.bbbbbbbbbbbb".into(),
                    content: "[med unresolved_task] still need to migrate auth.".into(),
                    namespace: Some(super::CONVERSATION_MEMORY_NAMESPACE.to_string()),
                    category: MemoryCategory::Conversation,
                    timestamp: "2026-04-22T00:00:00Z".into(),
                    session_id: None,
                    score: Some(0.9),
                },
            ]);

        let loader = DefaultMemoryLoader::default();
        let out = loader
            .load_context(&mem, "what should I default to for storage?")
            .await
            .expect("loader must succeed");

        assert!(
            out.contains("[Prior conversations]"),
            "expected prior conversations block, got:\n{out}"
        );
        assert!(out.contains("Postgres"));
        assert!(
            !out.contains("migrate auth"),
            "med-importance entries must not auto-surface, got:\n{out}"
        );
        assert!(
            !out.contains("[provenance]"),
            "provenance is not rendered into the prompt block, got:\n{out}"
        );
    }

    #[tokio::test]
    async fn collect_recall_citations_filters_and_truncates_entries() {
        let mem = MockMemory::new(vec![
            entry("keep", "useful context", Some(0.9)),
            entry("drop", "too weak", Some(0.1)),
            entry("long", &"x".repeat(600), Some(0.8)),
        ]);

        let citations = collect_recall_citations(&mem, "hello", 5, 0.4)
            .await
            .expect("citation collection should succeed");
        assert_eq!(citations.len(), 2);
        assert_eq!(citations[0].key, "keep");
        assert_eq!(citations[1].key, "long");
        assert!(citations[1].snippet.ends_with("..."));
    }

    // ── Cross-chat context (#1505) ───────────────────────────────────────

    fn cross_chat_entry(
        cross_id: &str,
        session_id: &str,
        content: &str,
        score: Option<f64>,
    ) -> MemoryEntry {
        MemoryEntry {
            id: format!("episodic-cross:{cross_id}"),
            key: format!("{session_id}:user"),
            content: content.into(),
            namespace: None,
            category: MemoryCategory::Conversation,
            timestamp: "2026-05-15T00:00:00Z".into(),
            session_id: Some(session_id.into()),
            score,
        }
    }

    #[tokio::test]
    async fn loader_surfaces_cross_chat_block_with_provenance_tag() {
        let mut mem = MockMemory::new(Vec::new());
        mem.cross_chat = vec![cross_chat_entry(
            "1",
            "thread-source",
            "I prefer Postgres for new services",
            Some(0.9),
        )];

        let loader = DefaultMemoryLoader::default();
        let out = loader
            .load_context(&mem, "what database should I use?")
            .await
            .expect("loader must succeed");
        assert!(
            out.contains(CROSS_CHAT_HEADER.trim_end()),
            "expected cross-chat header, got:\n{out}"
        );
        assert!(
            out.contains("Postgres"),
            "expected the cross-chat fact in the loader output, got:\n{out}"
        );
        assert!(
            out.contains("[chat:"),
            "expected provenance tag, got:\n{out}"
        );
        assert!(
            !out.contains("thread-source"),
            "raw session id MUST NOT leak into the prompt — render only the hashed tag, got:\n{out}"
        );
    }

    #[tokio::test]
    async fn loader_caps_cross_chat_block_at_limit() {
        let mut mem = MockMemory::new(Vec::new());
        mem.cross_chat = (0..10)
            .map(|i| {
                cross_chat_entry(
                    &format!("{i}"),
                    &format!("thread-{i}"),
                    &format!("Cross-chat fact #{i}"),
                    Some(0.9),
                )
            })
            .collect();

        let loader = DefaultMemoryLoader::default();
        let out = loader
            .load_context(&mem, "Cross-chat fact")
            .await
            .expect("loader must succeed");
        let cross_lines = out.lines().filter(|l| l.starts_with("- [chat:")).count();
        assert!(
            cross_lines <= CROSS_CHAT_LIMIT,
            "loader cross-chat block must be capped at {CROSS_CHAT_LIMIT}, saw {cross_lines}"
        );
    }

    #[tokio::test]
    async fn loader_drops_cross_chat_below_relevance_threshold() {
        let mut mem = MockMemory::new(Vec::new());
        mem.cross_chat = vec![
            cross_chat_entry("1", "thread-a", "low score chat fact", Some(0.05)),
            cross_chat_entry("2", "thread-b", "high score chat fact", Some(0.9)),
        ];

        let loader = DefaultMemoryLoader::default();
        let out = loader
            .load_context(&mem, "fact")
            .await
            .expect("loader must succeed");
        assert!(
            out.contains("high score chat fact"),
            "high-relevance cross-chat must surface, got:\n{out}"
        );
        assert!(
            !out.contains("low score chat fact"),
            "low-relevance cross-chat must be filtered, got:\n{out}"
        );
    }

    #[tokio::test]
    async fn loader_returns_empty_when_no_cross_chat_or_other_blocks_match() {
        let mem = MockMemory::new(Vec::new());
        let loader = DefaultMemoryLoader::default();
        let out = loader
            .load_context(&mem, "anything")
            .await
            .expect("loader must succeed");
        assert!(
            !out.contains(CROSS_CHAT_HEADER.trim_end()),
            "no cross-chat hits must produce no header, got:\n{out}"
        );
    }

    /// Exercises the **primary** cross-chat path (JSONL scan via
    /// `ConversationStore`, not the `Memory::recall` fallback). Writes
    /// two threads through `ConversationStore`, wires `workspace_dir`
    /// into the loader, and asserts the prompt picks up the hit from
    /// the inactive thread with a redacted provenance tag.
    ///
    /// Production-critical because the fallback `MockMemory` path is
    /// what the other loader tests cover — this is the one users
    /// actually run.
    #[tokio::test]
    async fn loader_surfaces_jsonl_primary_path_with_workspace_dir() {
        use crate::openhuman::memory_conversations::{
            ConversationMessage, ConversationStore, CreateConversationThread,
        };

        let temp = tempfile::TempDir::new().expect("tempdir");
        let store = ConversationStore::new(temp.path().to_path_buf());

        // Chat A — durable fact lives here.
        store
            .ensure_thread(CreateConversationThread {
                parent_thread_id: None,
                id: "thread-a".to_string(),
                title: "Chat A".to_string(),
                created_at: "2026-04-10T12:00:00Z".to_string(),
                labels: None,
                personality_id: None,
            })
            .expect("ensure thread-a");
        store
            .append_message(
                "thread-a",
                ConversationMessage {
                    id: "m-a-1".to_string(),
                    content: "Remember: my project Phoenix uses Go and PostgreSQL.".to_string(),
                    message_type: "text".to_string(),
                    extra_metadata: serde_json::json!({}),
                    sender: "user".to_string(),
                    created_at: "2026-04-10T12:01:00Z".to_string(),
                },
            )
            .expect("append a");

        // Chat B — active chat (excluded by current_thread_id wiring is
        // not exercised here; we just verify the JSONL path surfaces
        // hits from other threads).
        store
            .ensure_thread(CreateConversationThread {
                parent_thread_id: None,
                id: "thread-b".to_string(),
                title: "Chat B".to_string(),
                created_at: "2026-04-10T13:00:00Z".to_string(),
                labels: None,
                personality_id: None,
            })
            .expect("ensure thread-b");

        // MockMemory's cross_chat list is empty — if the loader fell
        // back to the Memory::recall path we'd render nothing. Forcing
        // a JSONL primary hit proves the workspace_dir branch ran.
        let mem = MockMemory::new(Vec::new());
        let loader = DefaultMemoryLoader::new(5, 0.4).with_workspace_dir(temp.path().to_path_buf());

        let out = loader
            .load_context(&mem, "What database does my project Phoenix use")
            .await
            .expect("loader must succeed");

        assert!(
            out.contains(CROSS_CHAT_HEADER.trim_end()),
            "JSONL primary path must emit the cross-chat header, got:\n{out}"
        );
        assert!(
            out.contains("PostgreSQL"),
            "cross-chat block must carry the matched snippet, got:\n{out}"
        );
        assert!(
            out.contains("chat:"),
            "cross-chat block must render a `chat:<hash>` provenance tag, got:\n{out}"
        );
        assert!(
            !out.contains("thread-a"),
            "raw thread_id must not leak into the prompt, got:\n{out}"
        );
    }
}
