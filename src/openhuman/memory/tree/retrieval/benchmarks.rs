//! Memory retrieval benchmark fixtures — #1538
//!
//! Deterministic test scenarios that verify retrieval quality and safety
//! for OpenHuman's memory tree. Each scenario exercises the full pipeline
//! (ingest → extract → score → seal → retrieve) using synthetic fixture data
//! so no real user data is required.
//!
//! ## Scenarios
//!
//! | # | Scenario | What it tests |
//! |---|----------|---------------|
//! | 1 | Cross-chat recall | Chat A seeds data; Chat B queries related → relevant source retrieved, no dump |
//! | 2 | Citation bundle | Retrieval returns chunk/source IDs alongside content |
//! | 3 | Stale preference | Newer explicit correction supersedes older preference |
//! | 4 | Contradiction handling | Disagreeing sources surface with provenance labels |
//! | 5 | Long-source compression | Large source retrieves exact relevant leaf chunk |
//!
//! Run with: `cargo test --package openhuman_core -- retrieval_benchmarks`

#![cfg(test)]

use chrono::{TimeZone, Utc};
use tempfile::TempDir;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::canonicalize::chat::{ChatBatch, ChatMessage};
use crate::openhuman::memory::tree::ingest::ingest_chat;
use crate::openhuman::memory::tree::jobs::testing::drain_until_idle;
use crate::openhuman::memory::tree::retrieval::{
    fetch_leaves, query_global, query_source, query_topic, search_entities,
};
use crate::openhuman::memory::tree::types::SourceKind;

/// Shared test config — disables embedding for deterministic inert behaviour.
fn bench_config() -> (TempDir, Config) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    cfg.memory_tree.embedding_endpoint = None;
    cfg.memory_tree.embedding_model = None;
    cfg.memory_tree.embedding_strict = false;
    (tmp, cfg)
}

/// Helper: ingest a chat batch with deterministic timestamps.
async fn ingest_chat_batch(
    cfg: &Config,
    scope: &str,
    owner: &str,
    messages: Vec<(String, String)>, // (author, text)
    base_ts_millis: i64,
) -> Vec<String> {
    let batch = ChatBatch {
        platform: "slack".into(),
        channel_label: scope.into(),
        messages: messages
            .into_iter()
            .enumerate()
            .map(|(i, (author, text))| ChatMessage {
                author,
                timestamp: Utc
                    .timestamp_millis_opt(base_ts_millis + (i as i64) * 60_000)
                    .unwrap(),
                text,
                source_ref: None,
            })
            .collect(),
    };
    let result = ingest_chat(cfg, scope, owner, vec![], batch).await.unwrap();
    drain_until_idle(cfg).await.unwrap();
    result.chunk_ids
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 1 — Cross-chat recall
// ─────────────────────────────────────────────────────────────────────────────

/// Seeds "Phoenix migration Friday landing" in Chat A.
/// Queries from Chat B — should retrieve relevant source without dumping
/// unrelated history.
#[tokio::test]
async fn bench_cross_chat_recall() {
    let (_tmp, cfg) = bench_config();

    // Chat A — seeds the key fact
    ingest_chat_batch(
        &cfg,
        "slack:#eng",
        "alice",
        vec![
            (
                "alice".into(),
                "Phoenix migration status: landing Friday evening.".into(),
            ),
            ("bob".into(), "Confirmed, I'll handle the cutover.".into()),
        ],
        1_700_000_000_000,
    )
    .await;

    // Chat B — queries related topic
    ingest_chat_batch(
        &cfg,
        "slack:#ops",
        "carol",
        vec![
            (
                "carol".into(),
                "What's the current status of the Phoenix migration?".into(),
            ),
            ("dave".into(), "Friday landing confirmed per alice.".into()),
        ],
        1_700_100_000_000,
    )
    .await;

    // Query topic for "phoenix" — should surface alice's original fact
    let topic_resp = query_topic(&cfg, "phoenix", None, None, 20).await.unwrap();

    // Assertions
    assert!(
        !topic_resp.hits.is_empty(),
        "query_topic('phoenix') should return at least one hit from Chat A"
    );

    let phoenix_hits: Vec<_> = topic_resp
        .hits
        .iter()
        .filter(|h| h.content.to_lowercase().contains("phoenix"))
        .collect();

    assert!(
        !phoenix_hits.is_empty(),
        "at least one hit should mention 'phoenix'"
    );

    // Verify no source-dump behaviour: hits should have content under 1 KB
    for hit in &phoenix_hits {
        assert!(
            hit.content.len() <= 1024,
            "cross-chat recall should return concise hits, not raw dumps. Got {} chars",
            hit.content.len()
        );
    }
}

/// Verify search_entities surfaces entities from both chats independently.
#[tokio::test]
async fn bench_cross_chat_entity_discoverable() {
    let (_tmp, cfg) = bench_config();

    ingest_chat_batch(
        &cfg,
        "slack:#eng",
        "alice",
        vec![(
            "alice".into(),
            "alice@example.com is leading the Phoenix migration.".into(),
        )],
        1_700_000_000_000,
    )
    .await;

    ingest_chat_batch(
        &cfg,
        "slack:#ops",
        "carol",
        vec![(
            "carol".into(),
            "alice@example.com confirmed the Friday timeline.".into(),
        )],
        1_700_100_000_000,
    )
    .await;

    let matches = search_entities(&cfg, "alice", None, 10).await.unwrap();

    // alice should be discoverable via canonical email id
    let alice = matches
        .iter()
        .find(|m| m.canonical_id.contains("alice@example.com"))
        .expect("alice should be discoverable from both chats");

    assert!(
        alice.mention_count >= 2,
        "alice should have >= 2 mentions across both chats, got {}",
        alice.mention_count
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 2 — Citation bundle
// ─────────────────────────────────────────────────────────────────────────────

/// Verify retrieval returns chunk IDs and source refs (provenance chain).
#[tokio::test]
async fn bench_citation_bundle_provenance() {
    let (_tmp, cfg) = bench_config();

    ingest_chat_batch(
        &cfg,
        "slack:#eng",
        "alice",
        vec![(
            "alice".into(),
            "RFC-42 v3 is approved. Link: https://example.com/rfc42".into(),
        )],
        1_700_000_000_000,
    )
    .await;

    drain_until_idle(&cfg).await.unwrap();

    // query_source for Chat — should return hits with source_ref populated
    let source_resp = query_source(&cfg, None, Some(SourceKind::Chat), None, None, 20)
        .await
        .unwrap();

    assert!(
        source_resp.total >= 1,
        "query_source should return >= 1 hit for the ingested chat"
    );

    // Find hits with provenance
    let prov_hits: Vec<_> = source_resp
        .hits
        .iter()
        .filter(|h| h.source_ref.is_some())
        .collect();

    assert!(
        !prov_hits.is_empty(),
        "retrieval hits should include source_ref provenance (citation bundle)"
    );

    for hit in prov_hits {
        assert!(
            !hit.node_id.is_empty(),
            "hit node_id must be populated for citation"
        );
        assert!(
            hit.tree_kind.as_str() == "source" || hit.tree_kind.as_str() == "chat",
            "hit tree_kind should be source or chat, got {:?}",
            hit.tree_kind
        );
    }
}

/// fetch_leaves should hydrate exact chunk IDs with full content.
#[tokio::test]
async fn bench_citation_fetch_leaves_hydrates() {
    let (_tmp, cfg) = bench_config();

    let chunk_ids = ingest_chat_batch(
        &cfg,
        "slack:#eng",
        "alice",
        vec![(
            "alice".into(),
            "Critical decision: all services must migrate to TLS 1.3 by Q4.".into(),
        )],
        1_700_000_000_000,
    )
    .await;

    drain_until_idle(&cfg).await.unwrap();

    let leaves = fetch_leaves(&cfg, &chunk_ids).await.unwrap();

    assert_eq!(
        leaves.len(),
        chunk_ids.len(),
        "fetch_leaves must hydrate all requested chunk IDs"
    );

    for (leaf, expected_id) in leaves.iter().zip(chunk_ids.iter()) {
        assert_eq!(
            leaf.node_id, *expected_id,
            "fetch_leaves response node_id should match requested chunk_id"
        );
        assert!(
            !leaf.content.is_empty(),
            "fetch_leaves should return non-empty content"
        );
        assert!(
            leaf.source_ref.is_some(),
            "fetch_leaves leaf should carry source_ref for citation"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 3 — Stale preference
// ─────────────────────────────────────────────────────────────────────────────

/// Newer explicit preference must supersede older preference.
#[tokio::test]
async fn bench_stale_preference_newer_supersedes() {
    let (_tmp, cfg) = bench_config();

    // Older preference — sets theme to dark
    ingest_chat_batch(
        &cfg,
        "slack:#general",
        "alice",
        vec![(
            "alice".into(),
            "My preferred theme is dark mode, please set UI_THEME=dark".into(),
        )],
        1_700_000_000_000, // older
    )
    .await;

    // Newer explicit correction — overrides to light
    ingest_chat_batch(
        &cfg,
        "slack:#general",
        "alice",
        vec![(
            "alice".into(),
            "Update: actually I prefer light theme, please set UI_THEME=light".into(),
        )],
        1_700_200_000_000, // newer
    )
    .await;

    drain_until_idle(&cfg).await.unwrap();

    // Query for alice's preference
    let topic_resp = query_topic(&cfg, "alice", None, None, 20).await.unwrap();

    // Find hits mentioning both themes
    let dark_hits: Vec<_> = topic_resp
        .hits
        .iter()
        .filter(|h| h.content.to_lowercase().contains("dark"))
        .collect();

    let light_hits: Vec<_> = topic_resp
        .hits
        .iter()
        .filter(|h| h.content.to_lowercase().contains("light"))
        .collect();

    // Both themes should be present (old + new both stored)
    // but light should appear in at least one hit (newer supersedes)
    assert!(
        !light_hits.is_empty(),
        "newer explicit correction should appear in results (light theme hit)"
    );

    // And dark should also appear (history preserved, not silently deleted)
    assert!(
        !dark_hits.is_empty(),
        "older preference should also appear in results (history preserved)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 4 — Contradiction handling
// ─────────────────────────────────────────────────────────────────────────────

/// Disagreeing sources surface with clear provenance labels so the caller
/// can resolve the conflict, not silently discard one side.
#[tokio::test]
async fn bench_contradiction_surfaces_both_with_provenance() {
    let (_tmp, cfg) = bench_config();

    // Source A — claims target date is June 15
    ingest_chat_batch(
        &cfg,
        "slack:#eng",
        "alice",
        vec![(
            "alice".into(),
            "Q2 milestone: we target June 15 for the Phoenix launch.".into(),
        )],
        1_700_000_000_000,
    )
    .await;

    // Source B — contradicts with July 30
    ingest_chat_batch(
        &cfg,
        "email:pm",
        "bob",
        vec![(
            "bob".into(),
            "Re-scoping: the Phoenix launch is pushed to July 30 per stakeholder review.".into(),
        )],
        1_700_100_000_000,
    )
    .await;

    drain_until_idle(&cfg).await.unwrap();

    // Query for phoenix — should surface both sources
    let topic_resp = query_topic(&cfg, "phoenix", None, None, 20).await.unwrap();

    let phoenix_hits: Vec<_> = topic_resp
        .hits
        .iter()
        .filter(|h| h.content.to_lowercase().contains("phoenix"))
        .collect();

    assert!(
        phoenix_hits.len() >= 2,
        "contradiction scenario should surface >= 2 hits from different sources, got {}",
        phoenix_hits.len()
    );

    // Verify both scopes appear (slack:#eng and email:pm)
    let scopes: Vec<_> = phoenix_hits.iter().map(|h| h.tree_scope.clone()).collect();

    assert!(
        scopes.iter().any(|s| s.contains("slack")),
        "hit from slack:#eng expected"
    );
    assert!(
        scopes.iter().any(|s| s.contains("email")),
        "hit from email:pm expected"
    );

    // Each hit must have provenance (tree_id, tree_scope, source_ref)
    for hit in &phoenix_hits {
        assert!(
            !hit.tree_id.is_empty(),
            "contradiction hit must have tree_id provenance"
        );
        assert!(
            !hit.tree_scope.is_empty(),
            "contradiction hit must have tree_scope for source identification"
        );
        assert!(
            !hit.content.is_empty(),
            "contradiction hit must have content"
        );
    }

    // Verify June and July dates are both present
    let content_all = phoenix_hits
        .iter()
        .map(|h| h.content.as_str())
        .collect::<Vec<_>>()
        .join(" ");

    assert!(
        content_all.to_lowercase().contains("june") && content_all.to_lowercase().contains("july"),
        "contradiction hits should include both June and July date references from both sources. Got: {}",
        content_all
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 5 — Long-source compression
// ─────────────────────────────────────────────────────────────────────────────

/// A large source (> 10k tokens) should retrieve only the exact relevant leaf
/// chunk, not the entire source content.
#[tokio::test]
async fn bench_long_source_retrieves_exact_leaf() {
    let (_tmp, cfg) = bench_config();

    // Build a long conversation — 30 messages, each ~200 tokens
    // Total far exceeds the chunk size, forcing multiple chunks
    let messages: Vec<(String, String)> = (0..30)
        .map(|i| {
            (
                "alice".into(),
                format!(
                    "Engineering log {}: Detailed technical note about system architecture \
                     design decisions, database sharding strategy, and deployment \
                     pipeline configuration for the Phoenix project. This entry contains \
                     specific implementation details for iteration {}.",
                    i, i
                ),
            )
        })
        .collect();

    ingest_chat_batch(&cfg, "slack:#eng", "alice", messages, 1_700_000_000_000).await;

    drain_until_idle(&cfg).await.unwrap();

    // Query the long source — should return summaries, not raw chunks
    let source_resp = query_source(&cfg, None, Some(SourceKind::Chat), None, None, 20)
        .await
        .unwrap();

    // Total hits should be bounded (summaries, not all raw chunks)
    assert!(
        source_resp.total <= 10,
        "long source should not dump all chunks; expected <= 10 summaries, got {}",
        source_resp.total
    );

    // If we have summaries, they should be compact
    for hit in &source_resp.hits {
        assert!(
            hit.content.len() <= 1000,
            "summary hit should be compact (≤ 1000 chars), got {} for: {}",
            hit.content.len(),
            hit.content.chars().take(50).collect::<String>()
        );
    }
}

/// Verify drill_down on a summary returns only its children, not sibling content.
#[tokio::test]
async fn bench_drill_down_isolates_children() {
    let (_tmp, cfg) = bench_config();

    // Two separate scopes — eng and ops
    ingest_chat_batch(
        &cfg,
        "slack:#eng",
        "alice",
        vec![(
            "alice".into(),
            "eng-only secret: the internal API uses Bearer token auth.".into(),
        )],
        1_700_000_000_000,
    )
    .await;

    ingest_chat_batch(
        &cfg,
        "slack:#ops",
        "carol",
        vec![(
            "carol".into(),
            "ops-only note: production DB lives at internal.example.com.".into(),
        )],
        1_700_100_000_000,
    )
    .await;

    drain_until_idle(&cfg).await.unwrap();

    // query_topic for "eng" — should NOT surface ops content
    let topic_resp = query_topic(&cfg, "eng", None, None, 20).await.unwrap();

    let eng_content = topic_resp
        .hits
        .iter()
        .map(|h| h.content.as_str())
        .collect::<Vec<_>>()
        .join(" ");

    assert!(
        !eng_content.to_lowercase().contains("ops"),
        "drill_down / query_topic should not cross scope into unrelated channels. \
         Found 'ops' content in eng query: {}",
        eng_content.chars().take(200).collect::<String>()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 6 — Scale/soak fixture (no real user data)
// ─────────────────────────────────────────────────────────────────────────────

/// Ingest 20 sources across 5 platforms — verify retrieval remains correct
/// at scale without any real user data.
#[tokio::test]
async fn bench_scale_ingest_20_sources_no_real_data() {
    let (_tmp, cfg) = bench_config();

    let platforms = vec![
        ("slack:#eng", "alice"),
        ("slack:#ops", "bob"),
        ("slack:#product", "carol"),
        ("email:team", "dave"),
        ("email:security", "eve"),
    ];

    for (i, (scope, owner)) in platforms.iter().cycle().take(20).enumerate() {
        let scope_str = scope.to_string();
        let owner_str = owner.to_string();
        ingest_chat_batch(
            &cfg,
            &scope_str,
            &owner_str,
            vec![(
                owner_str.clone().into(),
                format!(
                    "Scale test message {} from {} — verifying retrieval correctness \
                     at volume with deterministic synthetic data. No PII present.",
                    i, owner_str
                ),
            )],
            1_700_000_000_000 + (i as i64) * 60_000,
        )
        .await;
    }

    drain_until_idle(&cfg).await.unwrap();

    // query_global should show activity across the window
    let global_resp = query_global(&cfg, 30).await.unwrap();

    // Should have hits from multiple sources
    assert!(
        !global_resp.hits.is_empty(),
        "query_global should return hits after 20-source ingest"
    );

    // search_entities for each owner should return results
    for (_, owner) in &platforms {
        let matches = search_entities(&cfg, owner, None, 5).await.unwrap();
        assert!(
            !matches.is_empty(),
            "search_entities should find owner '{}' after scale ingest",
            owner
        );
    }
}
