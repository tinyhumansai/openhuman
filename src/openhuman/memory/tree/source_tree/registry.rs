//! Tree registry — get-or-create for source trees (#709).
//!
//! The registry is the entry point for the ingest path to look up the
//! tree for a given (kind, scope). Phase 3a only touches source trees;
//! topic / global trees will reuse the same `(kind, scope)` convention
//! in Phases 3b / 3c.

use anyhow::Result;
use chrono::Utc;
use uuid::Uuid;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::source_tree::store;
use crate::openhuman::memory::tree::source_tree::types::{Tree, TreeKind, TreeStatus};

/// Look up the source tree for `scope`, or create a new one.
///
/// Scope format convention (Phase 3a): use the ingested chunk's
/// `metadata.source_id` verbatim, so re-ingesting the same Slack channel
/// or Gmail account keeps appending to the same tree.
pub fn get_or_create_source_tree(config: &Config, scope: &str) -> Result<Tree> {
    if let Some(existing) = store::get_tree_by_scope(config, TreeKind::Source, scope)? {
        log::debug!(
            "[source_tree::registry] found tree id={} scope={}",
            existing.id,
            scope
        );
        return Ok(existing);
    }

    let tree = Tree {
        id: new_tree_id(TreeKind::Source),
        kind: TreeKind::Source,
        scope: scope.to_string(),
        root_id: None,
        max_level: 0,
        status: TreeStatus::Active,
        created_at: Utc::now(),
        last_sealed_at: None,
    };
    match store::insert_tree(config, &tree) {
        Ok(()) => {
            log::info!(
                "[source_tree::registry] created source tree id={} scope={}",
                tree.id,
                scope
            );
            Ok(tree)
        }
        Err(err) if is_unique_violation(&err) => {
            // Race: another caller created a tree for the same scope
            // between our initial lookup and this insert. UNIQUE(kind,
            // scope) rejected our row; re-query and return the winner.
            log::debug!(
                "[source_tree::registry] UNIQUE race for scope={} — re-querying",
                scope
            );
            store::get_tree_by_scope(config, TreeKind::Source, scope)?.ok_or_else(|| {
                anyhow::anyhow!(
                    "UNIQUE violation on insert but no row found on re-query for scope {scope}"
                )
            })
        }
        Err(err) => Err(err),
    }
}

/// Return true if `err` represents a SQLite UNIQUE constraint violation.
/// Matches both the anyhow-wrapped rusqlite error text and the raw SQLite
/// error codes in case the wrapping chain is shorter.
fn is_unique_violation(err: &anyhow::Error) -> bool {
    if let Some(rusqlite_err) = err.downcast_ref::<rusqlite::Error>() {
        if let rusqlite::Error::SqliteFailure(sqlite_err, _) = rusqlite_err {
            return sqlite_err.code == rusqlite::ErrorCode::ConstraintViolation;
        }
    }
    // Fallback for chained/wrapped errors: scan the rendered message.
    let msg = format!("{err:#}");
    msg.contains("UNIQUE constraint failed")
}

fn new_tree_id(kind: TreeKind) -> String {
    format!("{}:{}", kind.as_str(), Uuid::new_v4())
}

/// Public id generator for summary nodes — exported so `bucket_seal` can
/// share the same format (kept separate for readability; both use UUID v4
/// suffixes to keep ids short but unambiguous).
pub fn new_summary_id(level: u32) -> String {
    format!("summary:L{}:{}", level, Uuid::new_v4())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        (tmp, cfg)
    }

    #[test]
    fn get_or_create_is_idempotent_on_scope() {
        let (_tmp, cfg) = test_config();
        let first = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
        let second = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(first.kind, TreeKind::Source);
        assert_eq!(first.status, TreeStatus::Active);
    }

    #[test]
    fn different_scopes_yield_different_trees() {
        let (_tmp, cfg) = test_config();
        let a = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
        let b = get_or_create_source_tree(&cfg, "gmail:user@example.com").unwrap();
        assert_ne!(a.id, b.id);
        assert_ne!(a.scope, b.scope);
    }

    #[test]
    fn tree_id_has_expected_prefix() {
        let id = new_tree_id(TreeKind::Source);
        assert!(id.starts_with("source:"));
        let sum_id = new_summary_id(3);
        assert!(sum_id.starts_with("summary:L3:"));
    }

    #[test]
    fn get_or_create_recovers_from_unique_race() {
        // Simulate the race by pre-inserting a tree under the same scope
        // with a different id. `get_or_create` must re-query and return
        // the pre-existing row, not bubble the UNIQUE error.
        let (_tmp, cfg) = test_config();
        let pre_existing = Tree {
            id: "source:preexisting".into(),
            kind: TreeKind::Source,
            scope: "slack:#eng".into(),
            root_id: None,
            max_level: 0,
            status: TreeStatus::Active,
            created_at: Utc::now(),
            last_sealed_at: None,
        };
        store::insert_tree(&cfg, &pre_existing).unwrap();

        // First call finds it via get_tree_by_scope (happy path — no race
        // triggered here). To hit the race branch we need a caller that
        // skips the lookup and goes straight to insert with a fresh id.
        // Simplest proxy: call get_or_create twice from this test thread;
        // the first creates, the second's UNIQUE would fire if the
        // lookup was ever elided. Instead we cover the race path directly
        // via `is_unique_violation` on a synthesised insert failure below.
        let got = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
        assert_eq!(got.id, "source:preexisting");

        // Direct coverage: a second insert with a different id for the
        // same scope must surface as UNIQUE and be detected.
        let dup = Tree {
            id: "source:would-collide".into(),
            ..pre_existing.clone()
        };
        let err = store::insert_tree(&cfg, &dup).unwrap_err();
        assert!(
            is_unique_violation(&err),
            "expected UNIQUE violation, got: {err:#}"
        );
    }
}
