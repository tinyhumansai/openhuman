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
use crate::openhuman::memory::tree::tree_source::store;
use crate::openhuman::memory::tree::tree_source::types::{Tree, TreeKind, TreeStatus};

/// Look up the source tree for `scope`, or create a new one.
///
/// Scope format convention (Phase 3a): use the ingested chunk's
/// `metadata.source_id` verbatim, so re-ingesting the same Slack channel
/// or Gmail account keeps appending to the same tree.
pub fn get_or_create_source_tree(config: &Config, scope: &str) -> Result<Tree> {
    if let Some(existing) = store::get_tree_by_scope(config, TreeKind::Source, scope)? {
        log::debug!(
            "[tree_source::registry] found tree id={} scope={}",
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
                "[tree_source::registry] created source tree id={} scope={}",
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
                "[tree_source::registry] UNIQUE race for scope={} — re-querying",
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
    use rand::Rng;
    let ms = chrono::Utc::now().timestamp_millis() as u64;
    let rand_tail: u32 = rand::thread_rng().gen();
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
        assert!(sum_id.starts_with("summary:"));
        // Time-first layout: the segment after `summary:` is a 13-digit
        // zero-padded ms timestamp, then `:L<level>-<8hex>`.
        assert!(sum_id.contains(":L3-"), "expected level suffix in {sum_id}");
    }

    #[test]
    fn summary_id_format_is_lexicographically_chronological() {
        // The prefix `summary:` is identical across all ids, so the
        // first character that differs is in the 13-digit ms field.
        // Comparing two synthesised ids built around the same ms +/- a
        // step proves the format sorts by time without depending on
        // wall-clock granularity in the test runner. We verify the
        // generator's _format_ (the contract), not the system clock.
        let earlier_ms: u64 = 1_700_000_000_000;
        let later_ms: u64 = 1_700_000_000_001;
        // Use a max-tail rand for the earlier id to prove the
        // millisecond field dominates over the random suffix.
        let earlier = format!("summary:{:013}:L1-{:08x}", earlier_ms, u32::MAX);
        let later = format!("summary:{:013}:L9-{:08x}", later_ms, 0u32);
        assert!(
            earlier < later,
            "expected {earlier} < {later} (ms must outrank level + tail)"
        );

        // Sanity: a real id from the live generator parses with the
        // same prefix shape so the contract above maps onto runtime
        // values, not just synthesised strings.
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
