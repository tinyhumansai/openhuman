//! Singleton registry for the global activity digest tree (#709, Phase 3b).
//!
//! Unlike source trees (one per `source_id`) the global tree is a true
//! singleton per workspace — scope is the literal string `"global"`. The
//! lookup and race-recovery pattern otherwise mirrors
//! `source_tree::registry::get_or_create_source_tree`.

use anyhow::Result;
use chrono::Utc;
use uuid::Uuid;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::global_tree::GLOBAL_SCOPE;
use crate::openhuman::memory::tree::source_tree::store;
use crate::openhuman::memory::tree::source_tree::types::{Tree, TreeKind, TreeStatus};

/// Return the workspace's singleton global tree, creating it lazily on
/// first call. Safe to call on every ingest; subsequent calls short-circuit
/// to the existing row.
pub fn get_or_create_global_tree(config: &Config) -> Result<Tree> {
    if let Some(existing) = store::get_tree_by_scope(config, TreeKind::Global, GLOBAL_SCOPE)? {
        log::debug!(
            "[global_tree::registry] found global tree id={}",
            existing.id
        );
        return Ok(existing);
    }

    let tree = Tree {
        id: new_global_tree_id(),
        kind: TreeKind::Global,
        scope: GLOBAL_SCOPE.to_string(),
        root_id: None,
        max_level: 0,
        status: TreeStatus::Active,
        created_at: Utc::now(),
        last_sealed_at: None,
    };
    match store::insert_tree(config, &tree) {
        Ok(()) => {
            log::info!("[global_tree::registry] created global tree id={}", tree.id);
            Ok(tree)
        }
        Err(err) if is_unique_violation(&err) => {
            // Another caller beat us to it between our initial lookup and
            // the insert. The UNIQUE(kind, scope) index caught it —
            // re-query and return the winner.
            log::debug!("[global_tree::registry] UNIQUE race for global tree — re-querying");
            store::get_tree_by_scope(config, TreeKind::Global, GLOBAL_SCOPE)?.ok_or_else(|| {
                anyhow::anyhow!(
                    "UNIQUE violation on global-tree insert but no row found on re-query"
                )
            })
        }
        Err(err) => Err(err),
    }
}

/// True when `err` wraps a SQLite UNIQUE constraint violation. Duplicated
/// from `source_tree::registry` to keep this module self-contained; the
/// two copies are ~5 lines and have the same shape.
fn is_unique_violation(err: &anyhow::Error) -> bool {
    if let Some(rusqlite::Error::SqliteFailure(sqlite_err, _)) =
        err.downcast_ref::<rusqlite::Error>()
    {
        return sqlite_err.code == rusqlite::ErrorCode::ConstraintViolation;
    }
    let msg = format!("{err:#}");
    msg.contains("UNIQUE constraint failed")
}

fn new_global_tree_id() -> String {
    format!("{}:{}", TreeKind::Global.as_str(), Uuid::new_v4())
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
    fn get_or_create_is_idempotent() {
        let (_tmp, cfg) = test_config();
        let first = get_or_create_global_tree(&cfg).unwrap();
        let second = get_or_create_global_tree(&cfg).unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(first.kind, TreeKind::Global);
        assert_eq!(first.scope, GLOBAL_SCOPE);
        assert_eq!(first.status, TreeStatus::Active);
    }

    #[test]
    fn global_tree_has_expected_id_prefix() {
        let id = new_global_tree_id();
        assert!(id.starts_with("global:"));
    }

    #[test]
    fn race_recovery_returns_existing_row() {
        // Pre-seed a global tree so the second `get_or_create` path exercises
        // the normal lookup branch; the UNIQUE-race branch is covered by the
        // shared `is_unique_violation` contract in `source_tree::registry`.
        let (_tmp, cfg) = test_config();
        let pre_existing = Tree {
            id: "global:preexisting".into(),
            kind: TreeKind::Global,
            scope: GLOBAL_SCOPE.into(),
            root_id: None,
            max_level: 0,
            status: TreeStatus::Active,
            created_at: Utc::now(),
            last_sealed_at: None,
        };
        store::insert_tree(&cfg, &pre_existing).unwrap();

        let got = get_or_create_global_tree(&cfg).unwrap();
        assert_eq!(got.id, "global:preexisting");

        // And a direct duplicate insert must fire UNIQUE, covering the
        // detector path this module depends on for race recovery.
        let dup = Tree {
            id: "global:would-collide".into(),
            ..pre_existing.clone()
        };
        let err = store::insert_tree(&cfg, &dup).unwrap_err();
        assert!(
            is_unique_violation(&err),
            "expected UNIQUE violation, got {err:#}"
        );
    }
}
