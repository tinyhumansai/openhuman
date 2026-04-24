//! `memory_tree_search_entities` — free-text LIKE search over the entity
//! index (Phase 4 / #710).
//!
//! The entity index (`mem_tree_entity_index`) is populated at ingest time
//! with one row per (entity, node) occurrence. This tool exposes it to the
//! LLM as a fuzzy-ish lookup: "I'm not sure if alice is the canonical id —
//! let me search". We group by canonical id so repeated mentions collapse
//! into a single [`EntityMatch`] with an aggregate count.
//!
//! Matching rules:
//! - Query is lowercased before binding into the `LIKE` parameters.
//! - We match either `entity_id LIKE '%q%'` (canonical-id substring) OR
//!   `surface LIKE '%q%'` (display-form substring).
//! - `kinds` narrows the match by `entity_kind IN (...)` when non-empty.
//! - Output is ordered by mention count DESC so the strongest matches
//!   surface first.

use anyhow::{Context, Result};
use rusqlite::params_from_iter;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::retrieval::types::EntityMatch;
use crate::openhuman::memory::tree::score::extract::EntityKind;
use crate::openhuman::memory::tree::store::with_connection;

const DEFAULT_LIMIT: usize = 5;
const MAX_LIMIT: usize = 100;

/// Search the entity index for canonical ids matching `query`.
///
/// Returns at most `limit` matches (default 5, clamped to 100). Each match
/// is aggregated across every row of the entity index so `mention_count`
/// reflects total occurrences regardless of which tree they came from.
pub async fn search_entities(
    config: &Config,
    query: &str,
    kinds: Option<Vec<EntityKind>>,
    limit: usize,
) -> Result<Vec<EntityMatch>> {
    let limit = normalise_limit(limit);
    // Blank/whitespace-only queries would turn into `LIKE '%%'` and dump the
    // entire entity index. Return empty early instead. Flagged on PR #831
    // CodeRabbit review.
    let query = query.trim();
    if query.is_empty() {
        log::debug!("[retrieval::search] empty query — returning no matches");
        return Ok(Vec::new());
    }

    // Log `query_len` rather than the query itself — the query can be an
    // email, a handle, or any PII.
    log::debug!(
        "[retrieval::search] search_entities query_len={} kinds={:?} limit={}",
        query.len(),
        kinds
            .as_ref()
            .map(|ks| ks.iter().map(|k| k.as_str()).collect::<Vec<_>>()),
        limit
    );

    let q_lower = query.to_lowercase();
    let kinds_owned = kinds.clone();
    let config_owned = config.clone();
    let rows = tokio::task::spawn_blocking(move || -> Result<Vec<EntityMatch>> {
        with_connection(&config_owned, |conn| {
            let pattern = format!("%{q_lower}%");
            let (sql, params) = build_sql_and_params(&pattern, kinds_owned.as_deref(), limit);
            let mut stmt = conn
                .prepare(&sql)
                .with_context(|| "search_entities: failed to prepare statement")?;
            let mapped = stmt
                .query_map(params_from_iter(params.iter()), row_to_match)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .with_context(|| "search_entities: failed to collect rows")?;
            Ok(mapped)
        })
    })
    .await
    .map_err(|e| anyhow::anyhow!("search_entities join error: {e}"))??;

    log::debug!("[retrieval::search] returning matches={}", rows.len());
    Ok(rows)
}

fn normalise_limit(limit: usize) -> usize {
    if limit == 0 {
        DEFAULT_LIMIT
    } else {
        limit.min(MAX_LIMIT)
    }
}

/// Build the SQL string + bound parameters. Kept in its own function so we
/// can unit-test the shape of the generated statement without a real DB.
fn build_sql_and_params(
    pattern: &str,
    kinds: Option<&[EntityKind]>,
    limit: usize,
) -> (String, Vec<rusqlite::types::Value>) {
    use rusqlite::types::Value;
    let mut sql = String::from(
        "SELECT
            entity_id,
            entity_kind,
            MAX(surface) AS surface_sample,
            COUNT(*) AS mention_count,
            MAX(timestamp_ms) AS last_seen_ms
         FROM mem_tree_entity_index
         WHERE (LOWER(entity_id) LIKE ?1 OR LOWER(surface) LIKE ?1)",
    );
    let mut params: Vec<Value> = vec![Value::Text(pattern.to_string())];

    if let Some(ks) = kinds {
        if !ks.is_empty() {
            let placeholders: Vec<String> = (0..ks.len()).map(|i| format!("?{}", i + 2)).collect();
            sql.push_str(&format!(
                " AND entity_kind IN ({})",
                placeholders.join(", ")
            ));
            for k in ks {
                params.push(Value::Text(k.as_str().to_string()));
            }
        }
    }

    sql.push_str(
        " GROUP BY entity_id, entity_kind
          ORDER BY mention_count DESC, last_seen_ms DESC
          LIMIT ?",
    );
    params.push(Value::Integer(limit as i64));

    (sql, params)
}

fn row_to_match(row: &rusqlite::Row<'_>) -> rusqlite::Result<EntityMatch> {
    let canonical_id: String = row.get(0)?;
    let kind_s: String = row.get(1)?;
    let surface: String = row.get(2)?;
    let mention_count: i64 = row.get(3)?;
    let last_seen_ms: i64 = row.get(4)?;

    let kind = EntityKind::parse(&kind_s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, e.into())
    })?;

    Ok(EntityMatch {
        canonical_id,
        kind,
        surface,
        mention_count: mention_count.max(0) as u64,
        last_seen_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::canonicalize::chat::{ChatBatch, ChatMessage};
    use crate::openhuman::memory::tree::ingest::ingest_chat;
    use chrono::{TimeZone, Utc};
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        // Phase 4 (#710): ingest in seeding needs inert embedder.
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        (tmp, cfg)
    }

    async fn seed_chat(cfg: &Config, source: &str, text: &str) {
        let batch = ChatBatch {
            platform: "slack".into(),
            channel_label: source.into(),
            messages: vec![ChatMessage {
                author: "alice".into(),
                timestamp: Utc.timestamp_millis_opt(1_700_000_000_000).unwrap(),
                text: text.into(),
                source_ref: Some("slack://x".into()),
            }],
        };
        ingest_chat(cfg, source, "alice", vec![], batch)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn empty_index_returns_empty_vec() {
        let (_tmp, cfg) = test_config();
        let matches = search_entities(&cfg, "alice", None, 10).await.unwrap();
        assert!(matches.is_empty());
    }

    #[tokio::test]
    async fn matches_on_entity_id_substring() {
        let (_tmp, cfg) = test_config();
        seed_chat(
            &cfg,
            "slack:#eng",
            "Planning the Phoenix migration on Friday. alice@example.com owns it.",
        )
        .await;
        let matches = search_entities(&cfg, "alice", None, 10).await.unwrap();
        assert!(
            matches
                .iter()
                .any(|m| m.canonical_id == "email:alice@example.com"),
            "expected alice's canonical id in matches; got {matches:?}"
        );
    }

    #[tokio::test]
    async fn matches_on_surface_substring() {
        let (_tmp, cfg) = test_config();
        seed_chat(
            &cfg,
            "slack:#eng",
            "Planning the Phoenix migration. alice@example.com owns it. Running the runbook again.",
        )
        .await;
        // "example.com" appears in surface but not in canonical_id alone.
        let matches = search_entities(&cfg, "example.com", None, 10)
            .await
            .unwrap();
        assert!(
            matches.iter().any(|m| m.canonical_id.contains("alice")),
            "surface-matched row must surface; got {matches:?}"
        );
    }

    #[tokio::test]
    async fn kind_filter_narrows_results() {
        let (_tmp, cfg) = test_config();
        seed_chat(
            &cfg,
            "slack:#eng",
            "Planning Phoenix. alice@example.com. #launch-q2 tagged. \
             And let's confirm with bob@example.com too. runbook review.",
        )
        .await;
        let only_hashtags = search_entities(&cfg, "launch", Some(vec![EntityKind::Hashtag]), 10)
            .await
            .unwrap();
        assert!(only_hashtags
            .iter()
            .all(|m| matches!(m.kind, EntityKind::Hashtag)));
    }

    #[tokio::test]
    async fn matches_aggregate_across_multiple_sources() {
        let (_tmp, cfg) = test_config();
        seed_chat(
            &cfg,
            "slack:#a",
            "Meeting 1 about Phoenix. alice@example.com attends. The migration runbook review proceeds.",
        )
        .await;
        seed_chat(
            &cfg,
            "slack:#b",
            "Meeting 2 about Phoenix. alice@example.com attends again. Launch date confirmed.",
        )
        .await;
        let matches = search_entities(&cfg, "alice", None, 10).await.unwrap();
        let alice = matches
            .iter()
            .find(|m| m.canonical_id == "email:alice@example.com")
            .expect("alice should be in matches");
        assert!(
            alice.mention_count >= 2,
            "expected at least 2 mentions aggregated across sources, got {}",
            alice.mention_count
        );
    }

    #[tokio::test]
    async fn limit_truncates_results() {
        let (_tmp, cfg) = test_config();
        seed_chat(
            &cfg,
            "slack:#eng",
            "Planning Phoenix. alice@example.com. bob@example.com. \
             charlie@example.com. dana@example.com. eric@example.com. Run the runbook.",
        )
        .await;
        let matches = search_entities(&cfg, "example.com", None, 2).await.unwrap();
        assert!(matches.len() <= 2);
    }

    #[test]
    fn build_sql_without_kinds_has_no_in_clause() {
        let (sql, _params) = build_sql_and_params("%a%", None, 5);
        assert!(sql.contains("LOWER(entity_id) LIKE"));
        assert!(!sql.contains("entity_kind IN"));
    }

    #[test]
    fn build_sql_with_kinds_adds_in_clause() {
        let kinds = vec![EntityKind::Email, EntityKind::Hashtag];
        let (sql, params) = build_sql_and_params("%x%", Some(&kinds), 5);
        assert!(sql.contains("entity_kind IN"));
        // pattern + 2 kinds + limit = 4 params
        assert_eq!(params.len(), 4);
    }

    #[test]
    fn zero_limit_defaults_to_five() {
        assert_eq!(normalise_limit(0), DEFAULT_LIMIT);
    }

    #[test]
    fn huge_limit_is_clamped() {
        assert_eq!(normalise_limit(10_000), MAX_LIMIT);
    }
}
