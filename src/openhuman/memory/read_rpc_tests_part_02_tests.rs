use super::*;

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
asserts chunk lifecycle through `read_chunk_row`, which reads via the bound driver"]
async fn reset_tree_preserves_raw_archive_and_source_registry() {
    let (_tmp, cfg) = test_config();
    let chunk_id = seed_slack_chunk_with_raw_archive(&cfg).await;
    let content_root = cfg.memory_tree_content_root();
    let raw_file = content_root
        .join("raw")
        .join("slack-conn-slack-1")
        .join("chats")
        .join("1700000000000_1700000000.000100.md");
    let source_file = content_root
        .join("raw")
        .join("slack-conn-slack-1")
        .join("_source.md");
    assert!(raw_file.exists(), "raw archive should exist before reset");
    assert!(
        source_file.exists(),
        "source registry should exist before reset"
    );

    let stale_summary = content_root
        .join("wiki")
        .join("summaries")
        .join("source-slack-conn-slack-1")
        .join("L1")
        .join("summary-stale.md");
    std::fs::create_dir_all(
        stale_summary
            .parent()
            .expect("stale summary parent should exist"),
    )
    .expect("create stale summary dir");
    std::fs::write(&stale_summary, "stale summary body").expect("write stale summary");
    assert!(stale_summary.exists(), "stale summary fixture should exist");

    let outcome = reset_tree_rpc(&cfg).await.expect("reset_tree");
    assert_eq!(outcome.value.chunks_requeued, 1);
    assert_eq!(outcome.value.jobs_enqueued, 1);
    assert!(
        outcome.value.tree_rows_deleted >= 1,
        "buffer/tree rows should be removed during reset"
    );

    let row = read_chunk_row(&chunk_id)
        .await
        .expect("read chunk row")
        .expect("chunk row present after reset");
    assert_eq!(row.lifecycle_status, "pending_extraction");
    assert!(raw_file.exists(), "raw archive must survive reset_tree");
    assert!(
        source_file.exists(),
        "source registry must survive reset_tree"
    );
    assert!(
        !content_root.join("wiki").join("summaries").exists(),
        "derived wiki summaries should be removed"
    );
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
chunk detail is read through the bound driver, not the in-process engine"]
async fn read_chunk_row_returns_none_for_missing_chunk() {
    let (_tmp, _cfg) = test_config();
    assert!(read_chunk_row("missing-chunk").await.unwrap().is_none());
}

#[test]
fn display_name_unslugs_email_thread_with_user_hint() {
    let name = display_name_for_source(
        "gmail:alice@example.com|bob@example.com",
        Some("alice@example.com"),
    );
    assert_eq!(name, "bob@example.com");
}

#[test]
fn display_name_falls_back_to_arrow_when_user_unknown() {
    let name = display_name_for_source("gmail:alice@example.com|bob@example.com", None);
    assert!(name.contains("alice@example.com"));
    assert!(name.contains("bob@example.com"));
    assert!(name.contains("↔"));
}

#[test]
fn display_name_strips_platform_prefix() {
    assert_eq!(
        display_name_for_source("slack:#engineering", None),
        "#engineering"
    );
}

#[test]
fn display_name_handles_multiple_participants_and_trimmed_hint() {
    let name = display_name_for_source(
        "gmail:Alice@Example.com|bob@example.com|carol@example.com",
        Some(" alice@example.com "),
    );
    assert_eq!(name, "bob@example.com, carol@example.com");
}

#[test]
fn display_name_handles_no_prefix() {
    assert_eq!(display_name_for_source("loose-id", None), "loose-id");
}

#[test]
fn sanitize_basename_replaces_windows_illegal_characters() {
    assert_eq!(
        sanitize_basename(r#"chat:slack/#eng\name*?"<>|"#),
        "chat-slack-#eng-name------"
    );
    assert_eq!(sanitize_basename("safe-name.md"), "safe-name.md");
}

#[test]
fn parse_source_kind_str_accepts_known_values_only() {
    assert_eq!(parse_source_kind_str("chat"), Some(SourceKind::Chat));
    assert_eq!(parse_source_kind_str("email"), Some(SourceKind::Email));
    assert_eq!(
        parse_source_kind_str("document"),
        Some(SourceKind::Document)
    );
    assert_eq!(parse_source_kind_str("unknown"), None);
}

/// The namespace clear is routed through the bound driver's key/value tier
/// (`kv_list` + `kv_delete`) instead of a `rusqlite::Connection` this handler
/// opened on a path it built itself. The claim is unchanged and is what the
/// raw read-back below still checks: only the composio namespace goes.
#[tokio::test]
async fn clear_composio_sync_state_removes_only_target_namespace() {
    let (_tmp, cfg) = test_config();
    // Created up front so the raw fixture rows below have a schema to land in;
    // the driver installed further down opens this same store.
    let _memory =
        UnifiedMemory::new(cfg.workspace_dir.as_path(), Arc::new(NoopEmbedding), None).unwrap();
    let db_path = cfg.workspace_dir.join("memory").join("memory.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();

    conn.execute(
        "INSERT INTO kv_namespace (namespace, key, value_json, updated_at)
         VALUES (?1, 'cursor', '{}', 1.0)",
        params![KV_NAMESPACE],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO kv_namespace (namespace, key, value_json, updated_at)
         VALUES ('other-namespace', 'cursor', '{}', 2.0)",
        [],
    )
    .unwrap();
    drop(conn);

    crate::openhuman::memory::test_support::install_tinycortex_for_test(&cfg);
    let removed = clear_composio_sync_state(&cfg).await.unwrap();
    assert_eq!(removed, 1);

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let composio_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM kv_namespace WHERE namespace = ?1",
            params![KV_NAMESPACE],
            |row| row.get(0),
        )
        .unwrap();
    let other_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM kv_namespace WHERE namespace = 'other-namespace'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(composio_count, 0);
    assert_eq!(other_count, 1);
}

#[tokio::test]
async fn tree_graph_includes_leaf_chunks_linked_to_their_summary() {
    let (_tmp, cfg) = test_config();
    // The forest and its leaves are read through `MemoryTree`, so the graph
    // needs a driver that serves that family — the null fallback does not.
    crate::openhuman::memory::test_support::install_tinycortex_for_test(&cfg);
    insert_tree_summary(&cfg, "tree-1", "slack:#eng", "summary:1:L1-aaa", 1);
    insert_chunk_with_parent(
        &cfg,
        "chunk-sealed",
        Some("summary:1:L1-aaa"),
        1_700_000_000_000,
        "first line of sealed chunk\nmore body",
    );
    insert_chunk_with_parent(
        &cfg,
        "chunk-orphan",
        None,
        1_700_000_000_001,
        "orphan chunk body",
    );

    let resp = graph_export_rpc(&cfg, GraphMode::Tree).await.unwrap().value;

    // 1 source root + 1 summary + 2 leaf chunks = 4 nodes.
    assert_eq!(
        resp.nodes.len(),
        4,
        "source root + summary + both leaf chunks"
    );

    let source_root = resp.nodes.iter().find(|n| n.kind == "source").unwrap();
    assert!(source_root.id.starts_with("source:"));

    let summary = resp.nodes.iter().find(|n| n.kind == "summary").unwrap();
    assert_eq!(summary.id, "summary:1:L1-aaa");
    // Orphan summary links to source root.
    assert_eq!(summary.parent_id.as_deref(), Some(source_root.id.as_str()));

    let sealed = resp.nodes.iter().find(|n| n.id == "chunk-sealed").unwrap();
    assert_eq!(sealed.kind, "chunk");
    assert_eq!(sealed.parent_id.as_deref(), Some("summary:1:L1-aaa"));
    assert_eq!(sealed.label, "first line of sealed chunk");

    let orphan = resp.nodes.iter().find(|n| n.id == "chunk-orphan").unwrap();
    assert!(
        orphan.parent_id.is_none(),
        "unsealed chunk has no parent → renders as an orphan node"
    );

    assert!(resp.edges.is_empty());
}

#[tokio::test]
async fn tree_graph_keeps_summaries_first_then_chunks() {
    let (_tmp, cfg) = test_config();
    crate::openhuman::memory::test_support::install_tinycortex_for_test(&cfg);
    insert_tree_summary(&cfg, "tree-1", "slack:#eng", "summary:1:L1-aaa", 1);
    insert_chunk_with_parent(
        &cfg,
        "chunk-1",
        Some("summary:1:L1-aaa"),
        1_700_000_000_000,
        "a chunk",
    );

    let resp = graph_export_rpc(&cfg, GraphMode::Tree).await.unwrap().value;
    // Source roots are emitted first, then summaries, then chunks — so a
    // budget truncation drops chunk tails, never the tree skeleton.
    assert_eq!(resp.nodes[0].kind, "source");
    assert!(resp.nodes.iter().any(|n| n.kind == "summary"));
    assert!(resp.nodes.iter().any(|n| n.kind == "chunk"));
}

/// Contacts mode selects chunks by entity *kind* and labels them from one
/// batched entity read.
///
/// The two chunks carry a different number of person rows on purpose. A reader
/// that indexed the flat `chunk_entities` result by position against the ids it
/// sent — the trap the contract's docs call out — would attribute the second
/// chunk's row to the first and still produce two edges, so an asymmetric
/// fixture is what makes the grouping observable.
#[tokio::test]
async fn contacts_graph_selects_person_chunks_and_groups_edges_by_chunk() {
    let (_tmp, cfg) = test_config();
    crate::openhuman::memory::test_support::install_tinycortex_for_test(&cfg);

    insert_chunk_with_parent(
        &cfg,
        "chunk-a",
        None,
        1_700_000_000_002,
        "alice and bob met",
    );
    insert_chunk_with_parent(&cfg, "chunk-b", None, 1_700_000_000_001, "carol shipped it");
    // No person row: this chunk must not reach the graph at all.
    insert_chunk_with_parent(&cfg, "chunk-c", None, 1_700_000_000_000, "no people here");

    insert_entity_row(
        &cfg,
        "person:alice",
        "chunk-a",
        "person",
        "Alice",
        1_700_000_000_002,
    );
    insert_entity_row(
        &cfg,
        "person:bob",
        "chunk-a",
        "person",
        "Bob",
        1_700_000_000_002,
    );
    insert_entity_row(
        &cfg,
        "person:carol",
        "chunk-b",
        "person",
        "Carol",
        1_700_000_000_001,
    );
    // A non-person row on a person-bearing chunk: it must not become an edge.
    insert_entity_row(
        &cfg,
        "topic:shipping",
        "chunk-b",
        "topic",
        "shipping",
        1_700_000_000_001,
    );

    let resp = graph_export_rpc(&cfg, GraphMode::Contacts)
        .await
        .unwrap()
        .value;

    let chunk_ids: Vec<&str> = resp
        .nodes
        .iter()
        .filter(|n| n.kind == "chunk")
        .map(|n| n.id.as_str())
        .collect();
    assert_eq!(
        chunk_ids,
        vec!["chunk-a", "chunk-b"],
        "only person-bearing chunks, newest first"
    );

    let mut edges: Vec<(&str, &str)> = resp
        .edges
        .iter()
        .map(|e| (e.from.as_str(), e.to.as_str()))
        .collect();
    edges.sort_unstable();
    assert_eq!(
        edges,
        vec![
            ("chunk-a", "person:alice"),
            ("chunk-a", "person:bob"),
            ("chunk-b", "person:carol"),
        ],
        "every edge names the chunk its row came from, and the topic row is filtered out"
    );

    let contacts: std::collections::BTreeSet<(&str, &str)> = resp
        .nodes
        .iter()
        .filter(|n| n.kind == "contact")
        .map(|n| (n.id.as_str(), n.label.as_str()))
        .collect();
    let expected: std::collections::BTreeSet<(&str, &str)> = [
        ("person:alice", "Alice"),
        ("person:bob", "Bob"),
        ("person:carol", "Carol"),
    ]
    .into_iter()
    .collect();
    assert_eq!(contacts, expected);
    assert!(resp
        .nodes
        .iter()
        .filter(|n| n.kind == "contact")
        .all(|n| n.entity_kind.as_deref() == Some("person")));
}

/// An empty candidate set must short-circuit, not become an unfiltered read.
///
/// The failure this pins is quiet: an empty predicate means *unfiltered* on
/// this seam, so a batch read handed the empty id list could answer with every
/// row in the store and the graph would fill with contacts for chunks it never
/// selected.
#[tokio::test]
async fn contacts_graph_with_no_person_chunks_is_empty() {
    let (_tmp, cfg) = test_config();
    crate::openhuman::memory::test_support::install_tinycortex_for_test(&cfg);
    insert_chunk_with_parent(&cfg, "chunk-a", None, 1_700_000_000_000, "no people here");
    insert_entity_row(
        &cfg,
        "topic:shipping",
        "chunk-a",
        "topic",
        "shipping",
        1_700_000_000_000,
    );

    let resp = graph_export_rpc(&cfg, GraphMode::Contacts)
        .await
        .unwrap()
        .value;

    assert!(resp.nodes.is_empty(), "nodes: {:?}", resp.nodes);
    assert!(resp.edges.is_empty(), "edges: {:?}", resp.edges);
}

#[tokio::test]
async fn obsidian_status_registered_when_override_config_lists_content_root() {
    let (_tmp, cfg) = test_config();
    let content_root = cfg.memory_tree_content_root();
    // A separate dir standing in for a non-standard Obsidian config
    // location, with an obsidian.json that registers the content root.
    let cfg_dir = TempDir::new().unwrap();
    let body = format!(
        "{{ \"vaults\": {{ \"id0\": {{ \"path\": {}, \"open\": true }} }} }}",
        serde_json::to_string(&content_root.to_string_lossy().to_string()).unwrap()
    );
    std::fs::write(cfg_dir.path().join("obsidian.json"), body).unwrap();

    let outcome =
        obsidian_vault_status_rpc(&cfg, Some(cfg_dir.path().to_string_lossy().to_string()))
            .await
            .unwrap();

    assert!(outcome.value.registered);
    assert!(outcome.value.config_found);
    assert_eq!(
        outcome.value.content_root_abs,
        content_root.to_string_lossy().to_string()
    );
    // The log reports the booleans but redacts the absolute path (it
    // embeds the user's home / username).
    assert!(
        outcome.logs[0].contains("registered=true"),
        "log: {}",
        outcome.logs[0]
    );
    assert!(
        !outcome.logs[0].contains(content_root.to_str().unwrap()),
        "log leaked content root: {}",
        outcome.logs[0]
    );
}

#[tokio::test]
async fn obsidian_status_not_registered_for_empty_override_dir() {
    let (_tmp, cfg) = test_config();
    // Empty override dir → no obsidian.json there → content root is not a
    // registered vault. (A temp content root can't be under any real host
    // vault either, so this stays false regardless of the dev machine.)
    let cfg_dir = TempDir::new().unwrap();
    let outcome =
        obsidian_vault_status_rpc(&cfg, Some(cfg_dir.path().to_string_lossy().to_string()))
            .await
            .unwrap();
    assert!(!outcome.value.registered);
}

#[tokio::test]
async fn obsidian_status_blank_override_is_treated_as_none() {
    // A whitespace-only override must be normalized to None rather than
    // resolving to "." and probing a stray local ./obsidian.json. The temp
    // content root isn't under any real host vault, so this stays false.
    let (_tmp, cfg) = test_config();
    let outcome = obsidian_vault_status_rpc(&cfg, Some("   ".to_string()))
        .await
        .unwrap();
    assert!(!outcome.value.registered);
}

#[tokio::test]
async fn vault_health_check_reports_missing_content_root_for_fresh_workspace() {
    // `pipeline_healthy` reads the process-global degraded flags (via
    // `pipeline_status_rpc` → `current_degraded_state`), which sibling
    // `memory_tree` tests set and never clear. Serialise + reset to a clean
    // baseline so the assertion is deterministic. See #4691.
    let _g = tinymemory_core::tree::health::test_guard();
    let (_tmp, cfg) = test_config();
    // `vault_health_check_rpc` folds in `pipeline_status_rpc`, which reads
    // through the bound driver. Bind an empty one explicitly: resolving the
    // real driver means loading the compiled module, which a test process
    // can block on rather than fail.
    crate::openhuman::memory::binding::install_diagnostics_for_test(
        &cfg.workspace_dir,
        &cfg.subsystems.memory,
        Default::default(),
        Default::default(),
    );
    let outcome = vault_health_check_rpc(&cfg, None).await.unwrap();

    assert!(!outcome.value.exists);
    assert!(!outcome.value.readable);
    assert!(!outcome.value.writable);
    assert!(!outcome.value.obsidian_registered);
    assert!(outcome.value.pipeline_healthy);
    assert_eq!(outcome.value.last_sync_ms, 0);
}

/// #4278: both vault RPCs stamp the core host's OS so a frontend attached
/// from a different OS can tell `content_root_abs` is a foreign-host path and
/// must not open/reveal it locally.
#[tokio::test]
async fn vault_rpcs_report_core_host_os() {
    let (_tmp, cfg) = test_config();
    // `vault_health_check_rpc` folds in `pipeline_status_rpc`, which reads
    // through the bound driver. Bind an empty one explicitly: resolving the
    // real driver means loading the compiled module, which a test process
    // can block on rather than fail.
    crate::openhuman::memory::binding::install_diagnostics_for_test(
        &cfg.workspace_dir,
        &cfg.subsystems.memory,
        Default::default(),
        Default::default(),
    );

    let status = obsidian_vault_status_rpc(&cfg, None).await.unwrap();
    assert_eq!(status.value.host_os, std::env::consts::OS);

    let health = vault_health_check_rpc(&cfg, None).await.unwrap();
    assert_eq!(health.value.host_os, std::env::consts::OS);
    assert!(
        !health.value.host_os.is_empty(),
        "host_os must be populated"
    );
}

#[tokio::test]
async fn vault_health_check_reports_writable_and_obsidian_registered_when_ready() {
    let (_tmp, cfg) = test_config();
    // `vault_health_check_rpc` folds in `pipeline_status_rpc`, and
    // `last_sync_ms` comes from the bound driver rather than from a `SELECT`
    // over the seeded chunk. The seed below still matters — it is what makes
    // `content_root` exist, which is what this test is actually about — but
    // the sync time has to come from a driver that reports one.
    crate::openhuman::memory::binding::install_diagnostics_for_test(
        &cfg.workspace_dir,
        &cfg.subsystems.memory,
        crate::openhuman::memory::api::provider::types::StoreStats {
            chunks: 1,
            chunks_with_structure: 0,
            most_recent_chunk_ms: Some(1_800_000_000_000),
        },
        Default::default(),
    );
    seed_chat_chunk(
        &cfg,
        "slack:#eng",
        "Vault health seed chunk so content_root exists and last_sync_ms > 0",
    )
    .await;

    let content_root = cfg.memory_tree_content_root();
    let cfg_dir = TempDir::new().unwrap();
    let body = format!(
        "{{ \"vaults\": {{ \"id0\": {{ \"path\": {}, \"open\": true }} }} }}",
        serde_json::to_string(&content_root.to_string_lossy().to_string()).unwrap()
    );
    std::fs::write(cfg_dir.path().join("obsidian.json"), body).unwrap();

    let outcome = vault_health_check_rpc(&cfg, Some(cfg_dir.path().to_string_lossy().to_string()))
        .await
        .unwrap();

    assert!(outcome.value.exists);
    assert!(outcome.value.readable);
    assert!(outcome.value.writable);
    assert!(outcome.value.obsidian_registered);
    // Intentionally NOT asserting `pipeline_healthy` here: with a seeded chunk
    // (total_chunks > 0) the derived status depends on the process-global
    // degraded flags, which unguarded parallel `memory_tree` extraction/pipeline
    // tests set and never clear (structure degrades only clears on a *successful*
    // extraction, which never happens under test). Post-#4691 a leaked "degraded"
    // correctly reads as unhealthy, so asserting healthy here would be flaky.
    // The health mapping is covered deterministically by the `pipeline_is_healthy`
    // unit tests in `read_rpc/vault.rs`; this test covers the filesystem readiness
    // wiring. See also `memory_tree::tree::rpc::pipeline_status_renders_the_drivers_chunk_aggregates`.
    assert!(outcome.value.last_sync_ms > 0);
    assert!(
        !outcome.logs[0].contains(content_root.to_str().unwrap()),
        "log leaked content root: {}",
        outcome.logs[0]
    );
}

/// Regression: `wipe_all` MUST also clear the source-ingest gate
/// (`mem_tree_ingested_sources`). Before the fix it cleared chunks/summaries
/// but left the gate claimed, so a wiped document source could never
/// re-ingest — the next sync saw `already_ingested` and wrote 0 chunks / 0
/// seal jobs. This pins that a wipe leaves the gate empty so re-sync works.
#[tokio::test]
async fn wipe_all_clears_ingest_gate() {
    use tinymemory_api::chunks::SourceKind;
    use tinymemory_core::store::chunks::store as chunk_store;

    let (_tmp, cfg) = test_config();
    // `wipe_all` asks the bound driver to purge; without one bound the workspace
    // resolves to the placeholder, which serves no Maintenance family and
    // refuses rather than reporting a wipe it did not do.
    crate::openhuman::memory::test_support::install_tinycortex_for_test(&cfg);
    let gate_key = "notion:conn-1:page-abc@1700000000000";

    // Claim the gate exactly as a document ingest does.
    chunk_store::with_connection(&cfg, |conn| {
        let tx = conn.unchecked_transaction()?;
        let claimed = chunk_store::claim_source_ingest_tx(
            &tx,
            SourceKind::Document,
            gate_key,
            1_700_000_000_000,
        )?;
        assert!(claimed, "first claim should succeed");
        tx.commit()?;
        Ok(())
    })
    .unwrap();
    assert!(
        chunk_store::is_source_ingested(&cfg, SourceKind::Document, gate_key).unwrap(),
        "gate must be claimed before wipe"
    );

    wipe_all_rpc(&cfg).await.expect("wipe_all_rpc");

    assert!(
        !chunk_store::is_source_ingested(&cfg, SourceKind::Document, gate_key).unwrap(),
        "wipe_all must clear mem_tree_ingested_sources so a wiped source can re-ingest"
    );
}
