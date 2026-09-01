use super::*;

/// #4 (full live seal): like the above, but the summary + on-disk file are
/// produced by the REAL `seal_one_level` pipeline (staged chunk body →
/// summarise → `stage_summary`), not hand-written. Then the REAL
/// `composio_delete_connection(clear_memory=true)` handler must cascade the
/// tree, the summary row, AND the seal-produced content file away.
#[tokio::test]
async fn composio_delete_connection_clear_memory_cascades_live_sealed_tree_and_file() {
    let _serialised = module_guard().await;
    use tinymemory_core::store::chunks::store::{
        get_summary_content_pointers, upsert_staged_chunks_tx,
    };
    use tinymemory_core::store::content::stage_chunks;
    use tinymemory_core::store::trees::store as tree_store;
    use tinymemory_core::store::trees::types::{Buffer, TreeKind};
    use tinymemory_core::tree::tree::bucket_seal::{seal_one_level, LabelStrategy};
    use tinymemory_core::tree_source::registry::get_or_create_source_tree;

    let app = Router::new()
        .route(
            "/agent-integrations/composio/connections",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"connections": [
                        {"id":"c1","toolkit":"slack","status":"ACTIVE"}
                    ]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/connections/{id}",
            axum::routing::delete(|Path(_id): Path<String>| async move {
                Json(json!({"success": true, "data": {"deleted": true}}))
            }),
        );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_with_backend(&tmp, base);
    // The memory clear-out runs through the bound driver now that it is routed
    // onto `forget_matching`, so the test has to bind one. TinyCortex is the
    // engine the loadable module wraps, and unlike the module it is not a
    // process singleton, so several of these can share one test binary.
    crate::openhuman::memory::test_support::install_tinycortex_for_test(&config);
    // Force the inert embedder so the real seal's summary-embed step doesn't
    // reach a live endpoint. `config_with_backend` stores a cloud session +
    // api_url, so the factory would otherwise build a *cloud* embedder against
    // the mock (no embeddings route). `embeddings_provider = "none"` is the
    // actual switch that selects `InertEmbedder`.
    config.embeddings_provider = Some("none".to_string());
    config.memory_tree.embedding_endpoint = None;
    config.memory_tree.embedding_model = None;
    config.memory_tree.embedding_strict = false;

    // Real chunk for slack:c1 WITH its body staged to disk, so the seal's
    // `hydrate_leaf_inputs` → `read_chunk_body` can resolve it.
    let chunk = sample_memory_chunk(SourceKind::Chat, "slack:c1", 0);
    memory_tree_store::upsert_chunks(&config, &[chunk.clone()]).expect("seed chunk");
    let staged = stage_chunks(
        &config.memory_tree_content_root(),
        std::slice::from_ref(&chunk),
    )
    .expect("stage chunk body");
    memory_tree_store::with_connection(&config, |conn| {
        let tx = conn.unchecked_transaction()?;
        upsert_staged_chunks_tx(&tx, &staged)?;
        tx.commit()?;
        Ok(())
    })
    .expect("record staged chunk pointer");

    // Run the REAL seal — produces a genuine summary row + on-disk file.
    let tree = get_or_create_source_tree(&config, "slack:c1").expect("source tree");
    let buf = Buffer {
        tree_id: tree.id.clone(),
        level: 0,
        item_ids: vec![chunk.id.clone()],
        token_sum: i64::from(chunk.token_count),
        oldest_at: Some(chunk.metadata.time_range.0),
    };
    memory_tree_store::with_connection(&config, |conn| {
        let tx = conn.unchecked_transaction()?;
        tree_store::upsert_buffer_tx(&tx, &buf)?;
        tx.commit()?;
        Ok(())
    })
    .expect("persist buffer snapshot");
    let summary_id = seal_one_level(&config, &tree, &buf, &LabelStrategy::Empty, false)
        .await
        .expect("real seal produces a summary");

    // The seal wrote a real on-disk content file for the summary.
    let (rel, _sha) = get_summary_content_pointers(&config, &summary_id)
        .unwrap()
        .expect("seal staged a summary content file");
    let abs = {
        let mut p = config.memory_tree_content_root();
        for c in rel.split('/') {
            p.push(c);
        }
        p
    };
    assert!(
        abs.exists(),
        "seal must have written a summary file on disk"
    );
    assert!(
        tree_store::get_tree_by_scope(&config, TreeKind::Source, "slack:c1")
            .unwrap()
            .is_some()
    );

    // ---- act: REAL handler, clear_memory=true ----
    let outcome = composio_delete_connection(&config, "c1", true)
        .await
        .unwrap();
    assert!(outcome.value.deleted);
    assert_eq!(outcome.value.memory_chunks_deleted, 1);

    // chunk, tree, summary row, and the seal-produced file are all gone.
    assert!(memory_tree_store::get_chunk(&config, &chunk.id)
        .unwrap()
        .is_none());
    assert!(
        tree_store::get_tree_by_scope(&config, TreeKind::Source, "slack:c1")
            .unwrap()
            .is_none()
    );
    assert!(tree_store::get_summary(&config, &summary_id)
        .unwrap()
        .is_none());
    assert!(
        !abs.exists(),
        "seal-produced summary file must be removed via the real handler cascade"
    );
}

#[tokio::test]
async fn composio_delete_connection_clear_memory_keeps_other_gmail_connections() {
    let _serialised = module_guard().await;
    let app = Router::new()
        .route(
            "/agent-integrations/composio/connections",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"connections": [
                        {"id":"c1","toolkit":"gmail","status":"ACTIVE"},
                        {"id":"c2","toolkit":"gmail","status":"ACTIVE"}
                    ]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/connections/{id}",
            axum::routing::delete(|Path(_id): Path<String>| async move {
                Json(json!({"success": true, "data": {"deleted": true}}))
            }),
        );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    // The memory clear-out runs through the bound driver now that it is routed
    // onto `forget_matching`, so the test has to bind one. TinyCortex is the
    // engine the loadable module wraps, and unlike the module it is not a
    // process singleton, so several of these can share one test binary.
    crate::openhuman::memory::test_support::install_tinycortex_for_test(&config);
    let c1_account = sample_memory_chunk_with_owner(
        SourceKind::Email,
        "gmail:pilot-at-example-dot-com",
        "gmail-sync:c1",
        0,
    );
    let c2_account = sample_memory_chunk_with_owner(
        SourceKind::Email,
        "gmail:pilot-at-example-dot-com",
        "gmail-sync:c2",
        1,
    );
    let c1_connection_scoped =
        sample_memory_chunk_with_owner(SourceKind::Email, "gmail:c1:thread-a", "gmail-sync:c1", 2);
    let c2_connection_scoped =
        sample_memory_chunk_with_owner(SourceKind::Email, "gmail:c2:thread-b", "gmail-sync:c2", 3);
    memory_tree_store::upsert_chunks(
        &config,
        &[
            c1_account,
            c2_account.clone(),
            c1_connection_scoped,
            c2_connection_scoped.clone(),
        ],
    )
    .expect("chunks should seed");

    let outcome = composio_delete_connection(&config, "c1", true)
        .await
        .unwrap();

    assert!(outcome.value.deleted);
    assert_eq!(outcome.value.memory_chunks_deleted, 2);
    let remaining = memory_tree_store::list_chunks(
        &config,
        &memory_tree_store::ListChunksQuery {
            source_kind: Some(SourceKind::Email),
            ..Default::default()
        },
    )
    .expect("chunks should list");
    assert_eq!(remaining.len(), 2);
    assert!(remaining.iter().any(|chunk| chunk.id == c2_account.id));
    assert!(remaining
        .iter()
        .any(|chunk| chunk.id == c2_connection_scoped.id));
}

#[tokio::test]
async fn notion_cleanup_targets_include_synced_page_sources() {
    // The embedding seam fails loudly when unwired. Installed here rather
    // than relied upon from another test: `install_for_tests` is
    // `Once`-guarded, so a test that omits it passes only while some
    // earlier test in the same binary happened to run first.
    crate::openhuman::memory::host_impls::install_for_tests();
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    // The cleanup targets are read back through the bound driver now, so the
    // test has to bind one — the writes below go through a client over the
    // same workspace, and an unbound config resolves to the null driver,
    // which serves nothing and would report no targets at all.
    crate::openhuman::memory::test_support::install_tinycortex_for_test(&config);
    let memory = std::sync::Arc::new(
        MemoryClient::from_workspace_dir(config.workspace_dir.clone())
            .expect("memory client should initialise"),
    );
    // tinymemory v1.13.4 deleted the whole in-process Composio pipeline —
    // `sync_state::PersistedSyncState`/`HostSyncAdapter` included — so there is
    // no extension trait to save through any more. `memory_cleanup.rs`'s reader
    // deserialises this row as `tinycortex::memory::sync::SyncState` (the same
    // shape tinycortex's own sync layer writes), so the test writes that type
    // straight through the KV store instead.
    let mut state = tinycortex::memory::sync::SyncState::new("notion", "conn-1");
    state.mark_synced("page-a@2026-01-01T00:00:00Z");
    state.mark_synced("page-b");
    memory
        .kv_set(
            Some(tinycortex::memory::sync::state::STATE_NAMESPACE),
            "notion:conn-1",
            &serde_json::to_value(&state).expect("sync state should serialize"),
        )
        .await
        .expect("sync state should save");

    let targets = composio_memory_targets_for_connection(&config, Some("notion"), "conn-1")
        .await
        .expect("notion cleanup targets should resolve");

    assert!(targets.contains(&MemoryCleanupTarget::Exact(
        SourceKind::Document,
        "notion:page-a".to_string()
    )));
    assert!(targets.contains(&MemoryCleanupTarget::Exact(
        SourceKind::Document,
        "notion:page-b".to_string()
    )));
    assert!(targets.contains(&MemoryCleanupTarget::Exact(
        SourceKind::Document,
        "composio-notion-page-page-a".to_string()
    )));
}

#[tokio::test]
async fn notion_cleanup_targets_surface_corrupt_sync_state() {
    // The embedding seam fails loudly when unwired. Installed here rather
    // than relied upon from another test: `install_for_tests` is
    // `Once`-guarded, so a test that omits it passes only while some
    // earlier test in the same binary happened to run first.
    crate::openhuman::memory::host_impls::install_for_tests();
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    // The cleanup targets are read back through the bound driver now, so the
    // test has to bind one — the writes below go through a client over the
    // same workspace, and an unbound config resolves to the null driver,
    // which serves nothing and would report no targets at all.
    crate::openhuman::memory::test_support::install_tinycortex_for_test(&config);
    let memory = std::sync::Arc::new(
        MemoryClient::from_workspace_dir(config.workspace_dir.clone())
            .expect("memory client should initialise"),
    );
    memory
        .kv_set(
            Some(tinycortex::memory::sync::state::STATE_NAMESPACE),
            "notion:conn-1",
            &serde_json::json!({ "toolkit": 42 }),
        )
        .await
        .expect("corrupt sync state should be written");

    let err = composio_memory_targets_for_connection(&config, Some("notion"), "conn-1")
        .await
        .expect_err("corrupt sync state should surface");

    assert!(err.to_string().contains("failed to load notion sync state"));
}

#[tokio::test]
async fn drive_cleanup_targets_are_connection_scoped() {
    // The embedding seam fails loudly when unwired; same reasoning as the
    // notion tests above.
    crate::openhuman::memory::host_impls::install_for_tests();
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    // The drive arm never touches the store, but discovery takes the caller's
    // client unconditionally — the parameter is the seam the notion tests
    // inject through.
    let drive_memory = std::sync::Arc::new(
        MemoryClient::from_workspace_dir(config.workspace_dir.clone())
            .expect("memory client should initialise"),
    );

    let targets = composio_memory_targets_for_connection(&config, Some("google_drive"), "conn-1")
        .await
        .expect("drive cleanup targets should resolve");

    assert!(targets.contains(&MemoryCleanupTarget::Exact(
        SourceKind::Document,
        "drive:conn-1".to_string()
    )));
    assert!(targets.contains(&MemoryCleanupTarget::Prefix(
        SourceKind::Document,
        "googledrive:conn-1:".to_string()
    )));
    assert!(targets.contains(&MemoryCleanupTarget::Prefix(
        SourceKind::Document,
        "google_drive:conn-1/".to_string()
    )));
}

#[tokio::test]
async fn composio_get_user_profile_via_mock_returns_provider_profile() {
    let _serialised = module_guard().await;
    // The embedding seam fails loudly when unwired. Installed here rather
    // than relied upon from another test: `install_for_tests` is
    // `Once`-guarded, so a test that omits it passes only while some
    // earlier test in the same binary happened to run first.
    crate::openhuman::memory::host_impls::install_for_tests();
    use crate::openhuman::config::TEST_ENV_LOCK;
    let _cache_guard = cache_guard();
    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // This test mutates BACKEND_URL below via EnvVarGuard, which races with
    // api::config / core::cli_tests / medulla::ops / medulla::resolve tests
    // that mutate the same process-global var under the crate-wide lock —
    // TEST_ENV_LOCK alone does not serialize against those. Hold both.
    let _backend_env_guard = crate::api::config::backend_env_test_lock();

    let app = Router::new()
        .route(
            "/agent-integrations/composio/connections",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"connections": [
                        {"id":"c1","toolkit":"gmail","status":"ACTIVE"}
                    ]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/execute",
            post(|Json(body): Json<Value>| async move {
                let action = body
                    .get("tool")
                    .and_then(Value::as_str)
                    .or_else(|| body.get("action").and_then(Value::as_str))
                    .unwrap_or("");
                let data = match action {
                    "GMAIL_GET_PROFILE" => json!({
                        "emailAddress": "pilot@example.com",
                        "displayName": "Phoenix Pilot",
                        "profileUrl": "https://mail.google.com/mail/u/0/#inbox"
                    }),
                    other => panic!("unexpected action: {other}"),
                };
                Json(json!({
                    "success": true,
                    "data": {
                        "successful": true,
                        "data": data,
                        "error": null
                    }
                }))
            }),
        );
    let base = start_mock_backend(app).await;
    // ProviderContext reloads the saved config and applies runtime env
    // overlays. Pin the backend override to the mock so CI's BACKEND_URL
    // cannot redirect this request to the hosted API.
    let _backend_url_guard = EnvVarGuard::set("BACKEND_URL", &base);
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let _workspace_env_guard = WorkspaceEnvGuard::set(tmp.path());
    config.save().await.unwrap();

    let outcome = composio_get_user_profile(&config, "c1").await.unwrap();

    assert_eq!(outcome.value.toolkit, "gmail");
    assert_eq!(outcome.value.connection_id.as_deref(), Some("c1"));
    assert_eq!(outcome.value.email.as_deref(), Some("pilot@example.com"));
    assert_eq!(outcome.value.display_name.as_deref(), Some("Phoenix Pilot"));
    assert!(outcome.logs.iter().any(|l| l.contains("gmail")));
}

#[tokio::test]
async fn composio_list_tools_via_mock_with_filter() {
    let _serialised = module_guard().await;
    let app = Router::new().route(
        "/agent-integrations/composio/tools",
        get(|Query(_q): Query<HashMap<String, String>>| async move {
            Json(json!({
                "success": true,
                "data": {"tools": [
                    {"type":"function","function":{"name":"GMAIL_SEND_EMAIL"}},
                    {"type":"function","function":{"name":"GMAIL_SEARCH"}}
                ]}
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let outcome = composio_list_tools(&config, Some(vec!["gmail".into()]), None)
        .await
        .unwrap();
    assert_eq!(outcome.value.tools.len(), 2);
}

#[tokio::test]
async fn composio_execute_via_mock_succeeds_and_logs_elapsed() {
    let _serialised = module_guard().await;
    let app = Router::new().route(
        "/agent-integrations/composio/execute",
        post(|Json(b): Json<Value>| async move {
            Json(json!({
                "success": true,
                "data": {
                    "data": {"echo": b["tool"]},
                    "successful": true,
                    "error": null,
                    "costUsd": 0.001
                }
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let outcome = composio_execute(&config, "GMAIL_SEND", Some(json!({"to": "a"})), None)
        .await
        .unwrap();
    assert!(outcome.value.successful);
    assert!(outcome
        .logs
        .iter()
        .any(|l| l.contains("executed GMAIL_SEND")));
}

#[tokio::test]
async fn composio_execute_via_mock_propagates_backend_error() {
    let _serialised = module_guard().await;
    let app = Router::new().route(
        "/agent-integrations/composio/execute",
        post(|| async { Json(json!({"success": false, "error": "rate limited"})) }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let err = composio_execute(&config, "ANY_TOOL", None, None)
        .await
        .unwrap_err();
    // The dispatcher (`execute_composio_action`) classifies transport
    // failures and prefixes them with `[composio:error:<class>] …`; ops.rs
    // preserves that prefix so the frontend formatter can parse the class.
    // For an unrecognised tool slug and a 502-shaped envelope the only
    // signal we get is the backend error text, so assert on its contents.
    assert!(err.starts_with("[composio:error:"), "got: {err}");
    assert!(err.contains("rate limited"), "got: {err}");
}

#[tokio::test]
async fn composio_sync_gmail_via_mock_ingests_records_and_updates_outcome() {
    let _serialised = module_guard().await;
    // The embedding seam fails loudly when unwired. Installed here rather
    // than relied upon from another test: `install_for_tests` is
    // `Once`-guarded, so a test that omits it passes only while some
    // earlier test in the same binary happened to run first.
    crate::openhuman::memory::host_impls::install_for_tests();
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    use crate::openhuman::config::TEST_ENV_LOCK;
    let _cache_guard = cache_guard();
    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // See composio_get_user_profile_via_mock_returns_provider_profile: this
    // test also mutates BACKEND_URL via EnvVarGuard below, which needs the
    // crate-wide lock to serialize against api::config / core::cli_tests /
    // medulla's tests on the same process-global var.
    let _backend_env_guard = crate::api::config::backend_env_test_lock();

    let app = Router::new()
        .route(
            "/agent-integrations/composio/connections",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"connections": [
                        {"id":"c1","toolkit":"gmail","status":"ACTIVE"}
                    ]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/execute",
            post(|Json(body): Json<Value>| async move {
                let action = body
                    .get("tool")
                    .and_then(Value::as_str)
                    .or_else(|| body.get("action").and_then(Value::as_str))
                    .unwrap_or("");
                let data = match action {
                    "GMAIL_GET_PROFILE" => json!({
                        "emailAddress": "pilot@example.com",
                        "displayName": "Phoenix Pilot"
                    }),
                    "GMAIL_FETCH_EMAILS" => json!({
                        "messages": [{
                            "messageId": "gmail-msg-1",
                            "threadId": "gmail-thread-1",
                            "sender": "captain@example.com",
                            "to": "pilot@example.com",
                            "subject": "Phoenix launch canary",
                            "messageTimestamp": "2024-06-01T12:00:00Z",
                            "labelIds": ["INBOX"],
                            "markdownFormatted": "Phoenix launch canary body for mock sync coverage.",
                            "payload": {}
                        }]
                    }),
                    other => panic!("unexpected action: {other}"),
                };
                Json(json!({
                    "success": true,
                    "data": {
                        "successful": true,
                        "data": data,
                        "error": null
                    }
                }))
            }),
        );
    let base = start_mock_backend(app).await;
    // The provider action reloads config with env overlays before executing.
    // Keep that reload on the mock even when the runner exports BACKEND_URL.
    let _backend_url_guard = EnvVarGuard::set("BACKEND_URL", &base);
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_with_backend(&tmp, base);
    config.memory_tree.embedding_strict = false;
    let _workspace_env_guard = WorkspaceEnvGuard::set(tmp.path());
    config.save().await.unwrap();
    // The sync writes through the bound driver now, so the fixture binds one and
    // the read-back goes to the same place. Binding the global slot instead would
    // have the test write to one client and read from another — zero documents,
    // looking exactly like a sync that silently did nothing.
    crate::openhuman::memory::test_support::install_tinycortex_for_test(&config);
    // And the seams are re-installed against THIS config, not the
    // `Config::default()` that `install_for_tests` latched. Proxied Composio
    // resolves its bearer through `ComposioHost::session_bearer`, which reads
    // the installed host config — a default one has no session, so the sync
    // would refuse before reaching the mock backend. The setters overwrite, so
    // calling this after the latched install is what points the seam at the
    // config carrying the mock's URL and token.
    crate::openhuman::memory::host_impls::install_memory_host_seams(std::sync::Arc::new(
        config.clone(),
    ));

    let outcome = composio_sync(&config, "c1", Some("manual".to_string()))
        .await
        .unwrap();

    assert_eq!(outcome.value.toolkit, "gmail");
    assert_eq!(outcome.value.connection_id.as_deref(), Some("c1"));
    // composio_sync is now spawn-and-return: the immediate envelope is a
    // "started" sentinel, and the actual ingestion runs on a detached
    // tokio task. items_ingested == 0 / finished_at_ms == 0 / summary
    // contains "started" are the contract of that sentinel.
    assert_eq!(
        outcome.value.items_ingested, 0,
        "spawn-and-return: items_ingested on the immediate envelope is a 'started' sentinel, not a final count"
    );
    assert_eq!(
        outcome.value.finished_at_ms, 0,
        "spawn-and-return: finished_at_ms == 0 means 'task spawned, not yet complete'"
    );
    assert!(
        outcome.value.summary.contains("started"),
        "expected spawn-and-return summary to mention 'started', got: {}",
        outcome.value.summary
    );

    // Poll for the spawned ingest task to write the records into memory.
    //
    // The namespace is `source:<source_id>` because the sync now hands its
    // records to the bound driver's `accept_source_items` rather than writing a
    // provider-shaped skill document itself. That is the whole point of the
    // split — the module reads, this crate ingests — so reading them back the
    // way memory files them is what proves the two halves met.
    let documents = {
        let mut documents = Vec::new();
        for _ in 0..50 {
            let binding = crate::openhuman::memory::binding::for_config(&config)
                .expect("the fixture bound a driver");
            documents = binding
                .provider()
                .as_documents()
                .expect("the bound driver serves documents")
                .list_documents(Some("source:gmail:c1"))
                .await
                .unwrap()
                .get("documents")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if !documents.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        documents
    };
    assert_eq!(
        documents.len(),
        1,
        "expected one ingested Gmail record after the spawned task drains"
    );
    let document = &documents[0];
    assert_eq!(document["title"], "Phoenix launch canary");
    // `external_sync` is what stops a third party's words being treated later
    // as the user's own. A sync that ingested without it would be worse than
    // one that ingested nothing.
    assert_eq!(document["taint"], "external_sync");
}
