//! Generic tree registry — get-or-create for any tree kind (#709).
//!
//! All three tree flavors (Source, Global, Topic) share `UNIQUE(kind, scope)`
//! and the same race-recovery dance — there is no reason for three copies.
//! Source-specific side-effects (writing the `_source.md` mirror) live in
//! the `sources::registry` wrapper rather than here.

use anyhow::Result;
use chrono::Utc;
use uuid::Uuid;

use crate::openhuman::config::Config;
use crate::openhuman::memory_store::trees::types::{Tree, TreeKind, TreeStatus};
use crate::openhuman::memory_tree::tree::store;

/// Generic get-or-create. All three tree flavors (Source, Global, Topic)
/// share UNIQUE(kind, scope) and the same race-recovery dance — there's
/// no reason for three copies.
///
/// Source-specific side-effects (writing the `_source.md` on-disk mirror)
/// are NOT performed here; callers that need them should go through
/// [`crate::openhuman::memory::tree_source::registry::get_or_create_source_tree`].
pub fn get_or_create_tree(config: &Config, kind: TreeKind, scope: &str) -> Result<Tree> {
    if let Some(existing) = store::get_tree_by_scope(config, kind, scope)? {
        log::debug!(
            "[tree::registry] found tree id={} kind={} scope={}",
            existing.id,
            kind.as_str(),
            scope
        );
        return Ok(existing);
    }

    let tree = Tree {
        id: new_tree_id(kind),
        kind,
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
                "[tree::registry] created tree id={} kind={} scope={}",
                tree.id,
                kind.as_str(),
                scope
            );
            Ok(tree)
        }
        Err(err) if is_unique_violation(&err) => {
            // Race: another caller created a tree for the same (kind, scope)
            // between our initial lookup and this insert. UNIQUE(kind, scope)
            // rejected our row; re-query and return the winner.
            log::debug!(
                "[tree::registry] UNIQUE race for kind={} scope={} — re-querying",
                kind.as_str(),
                scope
            );
            store::get_tree_by_scope(config, kind, scope)?.ok_or_else(|| {
                anyhow::anyhow!(
                    "UNIQUE violation on insert but no row found on re-query for kind={} scope={}",
                    kind.as_str(),
                    scope
                )
            })
        }
        Err(err) => Err(err),
    }
}

/// Return true if `err` represents a SQLite UNIQUE constraint violation.
/// Matches both the anyhow-wrapped rusqlite error text and the raw SQLite
/// error codes in case the wrapping chain is shorter.
pub fn is_unique_violation(err: &anyhow::Error) -> bool {
    if let Some(rusqlite_err) = err.downcast_ref::<rusqlite::Error>() {
        if let rusqlite::Error::SqliteFailure(sqlite_err, _) = rusqlite_err {
            return sqlite_err.code == rusqlite::ErrorCode::ConstraintViolation;
        }
    }
    // Fallback for chained/wrapped errors: scan the rendered message.
    let msg = format!("{err:#}");
    msg.contains("UNIQUE constraint failed")
}

/// Generate a stable id for a new tree row, prefixed with the kind discriminator.
pub fn new_tree_id(kind: TreeKind) -> String {
    format!("{}:{}", kind.as_str(), Uuid::new_v4())
}

/// Public id generator for summary nodes — exported so `bucket_seal` can
/// share the same format. The Unix-ms timestamp is the leading sort
/// key so `ORDER BY id` is globally chronological across all levels
/// (a level-first layout grouped L1, L2, … together, breaking that).
/// `:013` zero-pads the millisecond field to 13 digits so the
/// lexicographic order matches numeric order through year 2286 — well
/// outside any reasonable retention window. Level is suffixed for
/// filter-by-level queries (`LIKE '%:L1-%'`). 8-hex of `u32` entropy
/// shrinks same-millisecond collision probability to ~2⁻³² per pair,
/// sized for uniqueness across the file-system and Obsidian wikilink
/// namespaces.
pub fn new_summary_id(level: u32) -> String {
    let ms = chrono::Utc::now().timestamp_millis() as u64;
    let rand_tail: u32 = rand::random();
    format!("summary:{:013}:L{}-{:08x}", ms, level, rand_tail)
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
        let first = get_or_create_tree(&cfg, TreeKind::Source, "slack:#eng").unwrap();
        let second = get_or_create_tree(&cfg, TreeKind::Source, "slack:#eng").unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(first.kind, TreeKind::Source);
        assert_eq!(first.status, TreeStatus::Active);
    }

    #[test]
    fn different_scopes_yield_different_trees() {
        let (_tmp, cfg) = test_config();
        let a = get_or_create_tree(&cfg, TreeKind::Source, "slack:#eng").unwrap();
        let b = get_or_create_tree(&cfg, TreeKind::Source, "gmail:user@example.com").unwrap();
        assert_ne!(a.id, b.id);
        assert_ne!(a.scope, b.scope);
    }

    #[test]
    fn different_kinds_same_scope_yield_different_trees() {
        let (_tmp, cfg) = test_config();
        let source = get_or_create_tree(&cfg, TreeKind::Source, "shared:scope").unwrap();
        let topic = get_or_create_tree(&cfg, TreeKind::Topic, "shared:scope").unwrap();
        assert_ne!(source.id, topic.id);
        assert_eq!(source.kind, TreeKind::Source);
        assert_eq!(topic.kind, TreeKind::Topic);
    }

    #[test]
    fn global_tree_is_singleton() {
        let (_tmp, cfg) = test_config();
        let first = get_or_create_tree(&cfg, TreeKind::Global, "global").unwrap();
        let second = get_or_create_tree(&cfg, TreeKind::Global, "global").unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(first.kind, TreeKind::Global);
    }

    #[test]
    fn tree_id_has_expected_prefix() {
        let source_id = new_tree_id(TreeKind::Source);
        assert!(source_id.starts_with("source:"));
        let topic_id = new_tree_id(TreeKind::Topic);
        assert!(topic_id.starts_with("topic:"));
        let global_id = new_tree_id(TreeKind::Global);
        assert!(global_id.starts_with("global:"));

        let sum_id = new_summary_id(3);
        assert!(sum_id.starts_with("summary:"));
        assert!(sum_id.contains(":L3-"), "expected level suffix in {sum_id}");
    }

    #[test]
    fn summary_id_format_is_lexicographically_chronological() {
        let earlier_ms: u64 = 1_700_000_000_000;
        let later_ms: u64 = 1_700_000_000_001;
        let earlier = format!("summary:{:013}:L1-{:08x}", earlier_ms, u32::MAX);
        let later = format!("summary:{:013}:L9-{:08x}", later_ms, 0u32);
        assert!(
            earlier < later,
            "expected {earlier} < {later} (ms must outrank level + tail)"
        );

        let live = new_summary_id(2);
        assert!(live.starts_with("summary:"), "live: {live}");
        let rest = &live["summary:".len()..];
        let ms_part = rest.split(':').next().expect("ms segment");
        assert_eq!(ms_part.len(), 13, "ms must be 13 digits in {live}");
        assert!(
            ms_part.chars().all(|c| c.is_ascii_digit()),
            "ms must be all digits in {live}"
        );
    }

    #[test]
    fn get_or_create_recovers_from_unique_race() {
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

        let got = get_or_create_tree(&cfg, TreeKind::Source, "slack:#eng").unwrap();
        assert_eq!(got.id, "source:preexisting");

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
