//! End-to-end: clean → compose → push the conversation into a tree as one
//! leaf. Uses the [`crate::openhuman::memory_tree`] write contract so the
//! archivist stays unaware of tree internals.
//!
//! The archivist intentionally writes one leaf per archived conversation
//! rather than persisting another bespoke store. `chunk_id_for_session`
//! hashes `(session_id, composed_markdown)` so retries are deterministic for
//! the same conversation snapshot while distinct sessions or edits produce a
//! fresh leaf id.
//!
//! These archivist leaves are synthetic conversation snapshots, not
//! `mem_tree_chunks` rows. That means they currently participate in the L0
//! buffer contract only: downstream source-tree sealing still expects
//! chunk-store-backed leaves when rehydrating inputs. Multi-conversation
//! summarisation for archivist-only source trees will need a dedicated
//! hydration path before these synthetic leaves can seal upward.

use anyhow::Result;
use chrono::Utc;
use sha2::{Digest, Sha256};

use crate::openhuman::config::Config;
use crate::openhuman::memory_archivist::clip::clean_conversation;
use crate::openhuman::memory_archivist::compose::compose_conversation_md;
use crate::openhuman::memory_archivist::types::Turn;
use crate::openhuman::memory_store::trees::{Tree, TreeKind};
use crate::openhuman::memory_tree::io::{
    TreeLabelStrategy, TreeLeafPayload, TreeWriteOutcome, TreeWriteRequest,
};
use crate::openhuman::memory_tree::tree::bucket_seal::{append_leaf, LabelStrategy};

const TOKEN_DIVISOR: usize = 4;

/// Clean the conversation, compose it as md, and append a single leaf to
/// the supplied tree. Returns the resulting [`TreeWriteOutcome`] including
/// any summary ids that sealed during the cascade.
pub async fn archive_to_tree(
    config: &Config,
    tree: &Tree,
    session_id: &str,
    turns: &[Turn],
) -> Result<TreeWriteOutcome> {
    let cleaned = clean_conversation(turns);
    let md = compose_conversation_md(&cleaned);
    let chunk_id = chunk_id_for_session(session_id, &md);
    let token_count = (md.len() / TOKEN_DIVISOR).max(1) as u32;
    let timestamp = cleaned.last().map(|t| t.timestamp).unwrap_or_else(Utc::now);

    let request = TreeWriteRequest {
        tree_id: tree.id.clone(),
        tree_kind: tree.kind,
        leaf: TreeLeafPayload {
            chunk_id: chunk_id.clone(),
            token_count,
            timestamp,
            content: md,
            entities: Vec::new(),
            topics: Vec::new(),
            score: 0.0,
        },
        label_strategy: TreeLabelStrategy::Inherit,
        deferred: false,
    };

    let leaf_ref = (&request.leaf).into();
    // Cleaned conversations have no extractor-derived entities/topics
    // riding along, so the only meaningful strategy is `Empty`. Callers
    // that want extraction can extend memory_tree::io::TreeLabelStrategy
    // and the dispatch below.
    let _ = request.label_strategy;
    let strategy = LabelStrategy::Empty;
    let new_summary_ids = append_leaf(config, tree, &leaf_ref, &strategy).await?;
    log::debug!(
        "[memory_archivist] archive_to_tree tree_id={} session={} chunk_id={} new_summaries={}",
        tree.id,
        session_id,
        chunk_id,
        new_summary_ids.len()
    );
    Ok(TreeWriteOutcome {
        new_summary_ids,
        seal_pending: false,
    })
}

fn chunk_id_for_session(session_id: &str, md: &str) -> String {
    let mut h = Sha256::new();
    h.update(session_id.as_bytes());
    h.update(b"\0");
    h.update(md.as_bytes());
    let digest = h.finalize();
    let hex = hex::encode(digest);
    format!("archivist:{}", &hex[..32])
}

// Kind helper so callers don't have to import TreeKind themselves when
// they pass a `Tree` they already have. (Re-export for ergonomic match.)
#[allow(dead_code)]
fn _kind_compile_check(t: &Tree) -> TreeKind {
    t.kind
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use tempfile::TempDir;

    use super::{archive_to_tree, chunk_id_for_session};
    use crate::openhuman::config::Config;
    use crate::openhuman::memory::tree_source::registry::get_or_create_source_tree;
    use crate::openhuman::memory_archivist::types::Turn;
    use crate::openhuman::memory_store::trees::store as tree_store;
    use crate::openhuman::memory_store::trees::{Tree, TreeKind, TreeStatus};

    #[test]
    fn chunk_id_is_stable_for_same_session_and_markdown() {
        let a = chunk_id_for_session("session-1", "## user\nhello\n");
        let b = chunk_id_for_session("session-1", "## user\nhello\n");
        assert_eq!(a, b);
        assert!(a.starts_with("archivist:"));
    }

    #[test]
    fn chunk_id_changes_when_session_changes() {
        let a = chunk_id_for_session("session-1", "## user\nhello\n");
        let b = chunk_id_for_session("session-2", "## user\nhello\n");
        assert_ne!(a, b);
    }

    #[test]
    fn chunk_id_changes_when_markdown_changes() {
        let a = chunk_id_for_session("session-1", "## user\nhello\n");
        let b = chunk_id_for_session("session-1", "## user\nhello again\n");
        assert_ne!(a, b);
    }

    fn test_config(tmp: &TempDir) -> Config {
        Config {
            workspace_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        }
    }

    fn source_tree(scope: &str) -> Tree {
        Tree {
            id: format!("tree:{scope}"),
            kind: TreeKind::Source,
            scope: scope.to_string(),
            root_id: None,
            max_level: 0,
            status: TreeStatus::Active,
            created_at: chrono::Utc::now(),
            last_sealed_at: None,
        }
    }

    #[tokio::test]
    async fn archive_to_tree_writes_a_leaf_for_conversation_turns() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        std::fs::create_dir_all(&cfg.workspace_dir).unwrap();
        let tree = get_or_create_source_tree(&cfg, "chat:slack:#eng").unwrap();

        let turns = vec![
            Turn {
                role: "user".into(),
                content: "How does ownership work in Rust?".into(),
                tool_calls_json: None,
                timestamp: chrono::Utc.with_ymd_and_hms(2026, 5, 24, 10, 0, 0).unwrap(),
            },
            Turn {
                role: "assistant".into(),
                content: "Ownership gives each value a single owner.".into(),
                tool_calls_json: Some("{\"tool\":\"ignored\"}".into()),
                timestamp: chrono::Utc.with_ymd_and_hms(2026, 5, 24, 10, 1, 0).unwrap(),
            },
        ];

        let outcome = archive_to_tree(&cfg, &tree, "session-1", &turns)
            .await
            .expect("archive_to_tree");
        assert!(
            outcome.new_summary_ids.is_empty(),
            "single archivist leaf should not seal summaries immediately"
        );
        assert!(!outcome.seal_pending);

        let buffer = tree_store::get_buffer(&cfg, &tree.id, 0).unwrap();
        assert_eq!(buffer.item_ids.len(), 1);
        let expected_md = "## user\nHow does ownership work in Rust?\n\n## assistant\nOwnership gives each value a single owner.\n";
        assert_eq!(
            buffer.item_ids[0],
            chunk_id_for_session("session-1", expected_md)
        );
        assert_eq!(
            buffer.token_sum,
            ((expected_md.len() / 4).max(1)) as i64,
            "token count should follow archivist TOKEN_DIVISOR heuristic"
        );
    }

    #[tokio::test]
    async fn archive_to_tree_handles_empty_turns_via_fallback_markdown() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        std::fs::create_dir_all(&cfg.workspace_dir).unwrap();
        let tree = get_or_create_source_tree(&cfg, "chat:empty").unwrap();

        let outcome = archive_to_tree(&cfg, &tree, "session-empty", &[])
            .await
            .expect("archive_to_tree empty");
        assert!(outcome.new_summary_ids.is_empty());

        let buffer = tree_store::get_buffer(&cfg, &tree.id, 0).unwrap();
        assert_eq!(buffer.item_ids.len(), 1);
        assert_eq!(
            buffer.item_ids[0],
            chunk_id_for_session("session-empty", ""),
            "empty conversation still generates a deterministic archivist chunk id"
        );
        assert_eq!(buffer.token_sum, 1);
    }

    #[tokio::test]
    async fn archive_to_tree_accumulates_multiple_sessions_in_buffer_order() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        std::fs::create_dir_all(&cfg.workspace_dir).unwrap();
        let tree = get_or_create_source_tree(&cfg, "chat:slack:#buffer-order").unwrap();

        let mut expected_ids = Vec::new();
        for idx in 0..3 {
            let turns = vec![Turn {
                role: "user".into(),
                content: format!("Conversation {idx} about the phoenix rollout."),
                tool_calls_json: None,
                timestamp: chrono::Utc
                    .with_ymd_and_hms(2026, 5, 24, 10, idx, 0)
                    .unwrap(),
            }];
            let outcome = archive_to_tree(&cfg, &tree, &format!("session-{idx}"), &turns)
                .await
                .expect("archive_to_tree multi-session batch");
            assert!(
                outcome.new_summary_ids.is_empty(),
                "archivist writes should remain buffered until a later seal-compatible path exists"
            );

            let expected_md = format!("## user\nConversation {idx} about the phoenix rollout.\n");
            expected_ids.push(chunk_id_for_session(
                &format!("session-{idx}"),
                &expected_md,
            ));
        }

        let l0 = tree_store::get_buffer(&cfg, &tree.id, 0).unwrap();
        assert_eq!(l0.item_ids, expected_ids);
        assert_eq!(l0.item_ids.len(), 3);
        assert!(
            l0.token_sum >= 3,
            "each archivist conversation contributes at least one token"
        );
    }
}
