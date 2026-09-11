use super::*;

// ── query_source_rpc ──────────────────────────────────────────────

/// An unknown kind is rejected **before** a driver is resolved, which is why
/// this test installs no binding: a caller mistake must not need a working
/// driver to be reported, and must not reach one and come back as an empty
/// store instead.
#[tokio::test]
async fn query_source_rpc_rejects_invalid_source_kind() {
    let (_tmp, cfg) = test_config();
    let req = QuerySourceRequest {
        source_id: None,
        source_kind: Some("bogus".into()),
        time_window_days: None,
        query: None,
        limit: None,
    };
    let err = query_source_rpc(&cfg, req).await.unwrap_err();
    assert!(err.contains("unknown source kind: bogus"), "got {err}");
}

/// The read degrades rather than fails when the bound driver has no
/// retrieval family, and still logs the count it served — a silent empty
/// and a degraded empty look identical downstream otherwise.
#[tokio::test]
async fn query_source_rpc_degrades_to_empty_without_the_retrieval_family() {
    let (_tmp, cfg) = test_config();
    bind_without_retrieval(&cfg);
    let outcome = query_source_rpc(&cfg, QuerySourceRequest::default())
        .await
        .expect("a driver without the retrieval family is not an error");
    assert!(outcome.value.hits.is_empty());
    assert_eq!(outcome.value.total, 0);
    assert_eq!(outcome.logs.len(), 1);
    let log = &outcome.logs[0];
    assert!(log.contains("has_source_id=false"), "log: {log}");
    assert!(log.contains("source_kind=None"), "log: {log}");
    assert!(log.contains("has_query=false"), "log: {log}");
    assert!(log.contains("hits=0"), "log: {log}");
}

#[tokio::test]
async fn query_source_rpc_redacts_source_id_from_its_log() {
    let (_tmp, cfg) = test_config();
    bind_without_retrieval(&cfg);
    let req = QuerySourceRequest {
        source_id: Some("slack:#eng".into()),
        source_kind: Some("chat".into()),
        time_window_days: None,
        query: None,
        limit: Some(5),
    };
    let outcome = query_source_rpc(&cfg, req).await.unwrap();
    assert!(outcome.value.hits.is_empty());
    let log = &outcome.logs[0];
    assert!(log.contains("has_source_id=true"), "log: {log}");
    assert!(log.contains("source_kind=Some(\"chat\")"), "log: {log}");
    // PII redaction: the raw source_id must NOT leak into the log.
    assert!(!log.contains("slack:#eng"), "log leaked source_id: {log}");
}

/// The filters reach the contract intact, and so does the turn's source
/// allowlist — the gate this handler is the only thing applying.
#[tokio::test]
async fn query_source_rpc_forwards_its_filters_and_the_turn_scope() {
    let (_tmp, cfg) = test_config();
    let driver = bind_recording(&cfg, RecordingRetrieval::new());
    let req = QuerySourceRequest {
        source_id: Some("slack:#eng".into()),
        source_kind: Some("chat".into()),
        time_window_days: Some(7),
        query: Some("phoenix".into()),
        limit: Some(5),
    };
    with_source_scope(Some(vec!["slack:#eng".into()]), async {
        query_source_rpc(&cfg, req).await.unwrap()
    })
    .await;

    let query = driver.source_query();
    assert_eq!(query.source_id.as_deref(), Some("slack:#eng"));
    assert_eq!(query.source_kind, Some(SourceKind::Chat));
    assert_eq!(query.time_window_days, Some(7));
    assert_eq!(query.query.as_deref(), Some("phoenix"));
    assert_eq!(query.limit, 5);

    let scope = driver
        .scope_for("retrieve_source")
        .expect("retrieve_source was called")
        .expect("a restricted turn must not reach the driver as unrestricted");
    assert_eq!(scope.allow, vec!["slack:#eng".to_string()]);
}

/// Outside a turn there is genuinely no restriction, and that has to travel
/// as `None`: an **empty** `SourceScope` denies every source-attributed row,
/// so mapping "unrestricted" onto one would blank recall out instead.
#[tokio::test]
async fn query_source_rpc_leaves_the_scope_absent_when_unrestricted() {
    let (_tmp, cfg) = test_config();
    let driver = bind_recording(&cfg, RecordingRetrieval::new());
    query_source_rpc(&cfg, QuerySourceRequest::default())
        .await
        .unwrap();
    assert_eq!(
        driver.scope_for("retrieve_source"),
        Some(None),
        "no ambient allowlist must reach the driver as None, not Some(empty)"
    );
}

/// An absent `limit` stays the engine's default rather than becoming a
/// request for zero rows.
#[tokio::test]
async fn query_source_rpc_maps_an_absent_limit_to_the_engine_sentinel() {
    let (_tmp, cfg) = test_config();
    let driver = bind_recording(&cfg, RecordingRetrieval::new());
    query_source_rpc(&cfg, QuerySourceRequest::default())
        .await
        .unwrap();
    assert_eq!(driver.source_query().limit, 0);
}

// ── cover_window_rpc ──────────────────────────────────────────────

#[tokio::test]
async fn cover_window_rpc_rejects_invalid_source_kind() {
    let (_tmp, cfg) = test_config();
    let req = CoverWindowRequest {
        since_ms: 0,
        until_ms: 1,
        source_id: None,
        source_kind: Some("bogus".into()),
        limit: None,
    };
    let err = cover_window_rpc(&cfg, req).await.unwrap_err();
    assert!(err.contains("cover_window:"), "got {err}");
    assert!(err.contains("unknown source kind: bogus"), "got {err}");
}

#[tokio::test]
async fn cover_window_rpc_degrades_to_empty_and_redacts_its_log() {
    let (_tmp, cfg) = test_config();
    bind_without_retrieval(&cfg);
    let req = CoverWindowRequest {
        since_ms: 0,
        until_ms: 4_000_000_000_000,
        source_id: Some("slack:#eng".into()),
        source_kind: Some("chat".into()),
        limit: None,
    };
    let outcome = cover_window_rpc(&cfg, req)
        .await
        .expect("a driver without the retrieval family is not an error");
    assert!(outcome.value.hits.is_empty());
    assert_eq!(outcome.value.total, 0);
    assert_eq!(outcome.logs.len(), 1);
    let log = &outcome.logs[0];
    assert!(log.contains("has_source_id=true"), "log: {log}");
    assert!(log.contains("source_kind=Some(\"chat\")"), "log: {log}");
    assert!(log.contains("hits=0"), "log: {log}");
    // PII redaction: the raw source_id must NOT leak into the log.
    assert!(!log.contains("slack:#eng"), "log leaked source_id: {log}");
}

#[tokio::test]
async fn cover_window_rpc_forwards_its_window_and_the_turn_scope() {
    let (_tmp, cfg) = test_config();
    let driver = bind_recording(&cfg, RecordingRetrieval::new());
    let req = CoverWindowRequest {
        since_ms: 10,
        until_ms: 20,
        source_id: Some("slack:#eng".into()),
        source_kind: Some("chat".into()),
        limit: Some(3),
    };
    with_source_scope(Some(vec!["slack:#eng".into()]), async {
        cover_window_rpc(&cfg, req).await.unwrap()
    })
    .await;

    let window = driver.window();
    assert_eq!(window.since_ms, 10);
    assert_eq!(window.until_ms, 20);
    assert_eq!(window.source_id.as_deref(), Some("slack:#eng"));
    assert_eq!(window.source_kind, Some(SourceKind::Chat));
    // Forwarded as sent: the driver, not this handler, maps absence onto
    // the engine's 0 sentinel.
    assert_eq!(window.limit, Some(3));

    let scope = driver
        .scope_for("cover_window")
        .expect("cover_window was called")
        .expect("a restricted turn must not reach the driver as unrestricted");
    assert_eq!(scope.allow, vec!["slack:#eng".to_string()]);
}

/// An inverted window is now the driver's rejection, and it has to arrive as
/// an error rather than as an empty page — the two are indistinguishable to
/// a caller otherwise, and one of them is a bug report.
#[tokio::test]
async fn cover_window_rpc_surfaces_a_driver_rejection() {
    let (_tmp, cfg) = test_config();
    bind_recording(
        &cfg,
        RecordingRetrieval::new().rejecting("until_ms 50 is before since_ms 100"),
    );
    let req = CoverWindowRequest {
        since_ms: 100,
        until_ms: 50,
        source_id: None,
        source_kind: None,
        limit: None,
    };
    let err = cover_window_rpc(&cfg, req).await.unwrap_err();
    assert!(err.contains("cover_window:"), "got {err}");
    assert!(err.contains("until_ms"), "got {err}");
    assert!(err.contains("since_ms"), "got {err}");
}

// ── search_entities_rpc ───────────────────────────────────────────

/// The search degrades rather than fails when the bound driver has no
/// retrieval family, and still logs the count it served — a silent empty
/// and a degraded empty look identical downstream otherwise.
#[tokio::test]
async fn search_entities_rpc_passes_through_kinds_none() {
    let (_tmp, cfg) = test_config();
    bind_without_retrieval(&cfg);
    let req = SearchEntitiesRequest {
        query: "alice".into(),
        kinds: None,
        limit: None,
    };
    let outcome = search_entities_rpc(&cfg, req)
        .await
        .expect("a driver without the retrieval family is not an error");
    assert!(outcome.value.matches.is_empty());
    let log = &outcome.logs[0];
    assert!(log.contains("query_len=5"), "log: {log}");
    assert!(log.contains("has_kinds=false"), "log: {log}");
    assert!(log.contains("n=0"), "log: {log}");
    // PII redaction — the raw query value must NOT appear in the log.
    assert!(!log.contains("alice"), "log leaked raw query: {log}");
}

#[tokio::test]
async fn search_entities_rpc_parses_valid_kinds_list() {
    let (_tmp, cfg) = test_config();
    bind_without_retrieval(&cfg);
    let req = SearchEntitiesRequest {
        query: "x".into(),
        kinds: Some(vec!["email".into(), "topic".into()]),
        limit: Some(10),
    };
    let outcome = search_entities_rpc(&cfg, req).await.unwrap();
    assert!(outcome.value.matches.is_empty());
    assert!(
        outcome.logs[0].contains("has_kinds=true"),
        "log: {}",
        outcome.logs[0]
    );
}

/// An unknown kind is rejected **before** a driver is resolved, which is why
/// this test installs no binding: a caller mistake must not need a working
/// driver to be reported, and must not reach one and come back as an empty
/// index instead.
#[tokio::test]
async fn search_entities_rpc_rejects_unknown_entity_kind() {
    // The handler reaches the bound driver, which refuses with "memory is
    // still starting" for as long as the module is loading — a process-wide
    // transient this assertion would otherwise race (openhuman#6172).
    crate::openhuman::memory::test_support::settle_memory_module().await;
    let (_tmp, cfg) = test_config();
    let req = SearchEntitiesRequest {
        query: "x".into(),
        kinds: Some(vec!["email".into(), "bogus".into()]),
        limit: None,
    };
    let err = search_entities_rpc(&cfg, req).await.unwrap_err();
    assert!(err.contains("unknown entity kind: bogus"), "got {err}");
}

// ── drill_down_rpc ────────────────────────────────────────────────

#[tokio::test]
async fn drill_down_rpc_defaults_max_depth_to_one_when_unset() {
    let (_tmp, cfg) = test_config();
    bind_without_retrieval(&cfg);
    let req = DrillDownRequest {
        node_id: "chat:missing".into(),
        max_depth: None,
        query: None,
        limit: None,
    };
    let outcome = drill_down_rpc(&cfg, req).await.unwrap();
    assert!(
        outcome.logs[0].contains("depth=1"),
        "log: {}",
        outcome.logs[0]
    );
}

#[tokio::test]
async fn drill_down_rpc_logs_node_kind_prefix_for_colon_separated_id() {
    let (_tmp, cfg) = test_config();
    bind_without_retrieval(&cfg);
    let req = DrillDownRequest {
        node_id: "chat:slack:#eng:0".into(),
        max_depth: Some(2),
        query: None,
        limit: None,
    };
    let outcome = drill_down_rpc(&cfg, req).await.unwrap();
    let log = &outcome.logs[0];
    assert!(log.contains("node_kind=chat"), "log: {log}");
    // PII redaction — scope segments beyond the kind prefix must not leak.
    assert!(!log.contains("slack"), "log leaked scope: {log}");
    assert!(!log.contains("#eng"), "log leaked scope: {log}");
}

#[tokio::test]
async fn drill_down_rpc_logs_unknown_when_node_id_has_no_colon() {
    let (_tmp, cfg) = test_config();
    bind_without_retrieval(&cfg);
    let req = DrillDownRequest {
        node_id: "rootnode".into(),
        max_depth: None,
        query: None,
        limit: None,
    };
    let outcome = drill_down_rpc(&cfg, req).await.unwrap();
    assert!(
        outcome.logs[0].contains("node_kind=unknown"),
        "log: {}",
        outcome.logs[0]
    );
}

/// A node id names a node, not a permission: the walk still has to run under
/// the turn's allowlist.
#[tokio::test]
async fn drill_down_rpc_forwards_the_turn_scope() {
    let (_tmp, cfg) = test_config();
    let driver = bind_recording(&cfg, RecordingRetrieval::new());
    let req = DrillDownRequest {
        node_id: "chat:slack:#eng:0".into(),
        max_depth: None,
        query: None,
        limit: None,
    };
    with_source_scope(Some(vec!["slack:#eng".into()]), async {
        drill_down_rpc(&cfg, req).await.unwrap()
    })
    .await;
    let scope = driver
        .scope_for("retrieve_children")
        .expect("retrieve_children was called")
        .expect("a restricted turn must not reach the driver as unrestricted");
    assert_eq!(scope.allow, vec!["slack:#eng".to_string()]);
}

// ── fetch_leaves_rpc ──────────────────────────────────────────────

#[tokio::test]
async fn fetch_leaves_rpc_returns_empty_response_for_empty_input() {
    let (_tmp, cfg) = test_config();
    bind_without_retrieval(&cfg);
    let req = FetchLeavesRequest { chunk_ids: vec![] };
    let outcome = fetch_leaves_rpc(&cfg, req).await.unwrap();
    assert!(outcome.value.hits.is_empty());
    assert!(outcome.logs[0].contains("n=0"), "log: {}", outcome.logs[0]);
}

/// Naming chunk ids directly must not read around the source gate, so the
/// scope travels with the ids.
#[tokio::test]
async fn fetch_leaves_rpc_returns_driver_hits_under_the_turn_scope() {
    let (_tmp, cfg) = test_config();
    let driver = bind_recording(
        &cfg,
        RecordingRetrieval::new().answering(vec![hit("chunk-a"), hit("chunk-b")]),
    );
    let req = FetchLeavesRequest {
        chunk_ids: vec!["chunk-a".into(), "chunk-b".into(), "ghost".into()],
    };
    let outcome = with_source_scope(Some(vec!["slack:#eng".into()]), async {
        fetch_leaves_rpc(&cfg, req).await.unwrap()
    })
    .await;
    assert_eq!(outcome.value.hits.len(), 2);
    assert!(outcome.logs[0].contains("n=2"), "log: {}", outcome.logs[0]);

    let scope = driver
        .scope_for("retrieve_leaves")
        .expect("retrieve_leaves was called")
        .expect("a restricted turn must not reach the driver as unrestricted");
    assert_eq!(scope.allow, vec!["slack:#eng".to_string()]);
}

/// `tree_kind` is `skip_serializing_if = "Option::is_none"`, so a hit that
/// lost its kind on the way through would take the key out of the response
/// rather than fail — the silent field loss that held this migration back
/// while the artifact predated the field.
#[tokio::test]
async fn retrieval_hits_keep_tree_kind_on_the_wire() {
    let (_tmp, cfg) = test_config();
    bind_recording(
        &cfg,
        RecordingRetrieval::new().answering(vec![hit("chunk-a")]),
    );
    let outcome = fetch_leaves_rpc(
        &cfg,
        FetchLeavesRequest {
            chunk_ids: vec!["chunk-a".into()],
        },
    )
    .await
    .unwrap();
    let json = serde_json::to_value(&outcome.value).unwrap();
    assert_eq!(json["hits"][0]["tree_kind"], serde_json::json!("source"));
}
