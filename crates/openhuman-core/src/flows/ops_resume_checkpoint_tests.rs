use super::*;

/// Rewrite a module-created run document only for the migration fixture below.
/// All normal run setup and execution still go through the TinyFlows bus.
pub(super) fn update_parked_run_document_for_test(
    config: &Config,
    run_id: &str,
    update: impl FnOnce(&mut Value),
) {
    use tinyflows_bus::catalog::{
        self as catalog, CatalogDocumentPrecondition, CatalogStorageReply,
        CatalogVersionedDocument, DocGetRequest, DocPutRequest,
    };

    let reply = crate::flows::catalog_storage::call_test_storage::<CatalogStorageReply, _>(
        config,
        catalog::storage::GET,
        (DocGetRequest {
            collection: "flows.runs".into(),
            id: run_id.to_owned(),
        },),
    )
    .expect("read the module-created parked run document");
    let value = match reply {
        CatalogStorageReply::Value { value } => value,
        CatalogStorageReply::Error { error } => panic!("run fixture read failed: {error:?}"),
    };
    let stored: Option<CatalogVersionedDocument> =
        serde_json::from_value(value).expect("decode the module-created parked run document");
    let mut stored = stored.expect("the parked run document must exist");
    update(&mut stored.doc);

    let reply = crate::flows::catalog_storage::call_test_storage::<CatalogStorageReply, _>(
        config,
        catalog::storage::PUT,
        (DocPutRequest {
            collection: "flows.runs".into(),
            id: run_id.to_owned(),
            doc: stored.doc,
            precondition: CatalogDocumentPrecondition::Version(stored.version),
            schema: stored.meta.schema,
            expires_ms: stored.meta.expires_ms,
            created_ms: Some(stored.meta.created_ms),
        },),
    )
    .expect("write the scoped migration fixture");
    assert!(matches!(reply, CatalogStorageReply::Value { .. }));
}

/// Corrupt a stored flow graph through the host's owner-scoped document port.
/// The malformed record is used to verify that one broken definition does not
/// block reconciliation of its valid siblings.
pub(super) fn corrupt_saved_flow_graph_for_test(config: &Config, flow_id: &str, graph: &str) {
    use tinyflows_bus::catalog::{
        self as catalog, CatalogDocumentPrecondition, CatalogStorageReply,
        CatalogVersionedDocument, DocGetRequest, DocPutRequest,
    };

    let reply = crate::flows::catalog_storage::call_test_storage::<CatalogStorageReply, _>(
        config,
        catalog::storage::GET,
        (DocGetRequest {
            collection: "flows.definitions".into(),
            id: flow_id.to_owned(),
        },),
    )
    .expect("read the module-created flow document");
    let value = match reply {
        CatalogStorageReply::Value { value } => value,
        CatalogStorageReply::Error { error } => panic!("flow fixture read failed: {error:?}"),
    };
    let stored: Option<CatalogVersionedDocument> =
        serde_json::from_value(value).expect("decode the module-created flow document");
    let mut stored = stored.expect("the flow document must exist");
    stored.doc["graph"] = Value::String(graph.to_owned());

    let reply = crate::flows::catalog_storage::call_test_storage::<CatalogStorageReply, _>(
        config,
        catalog::storage::PUT,
        (DocPutRequest {
            collection: "flows.definitions".into(),
            id: flow_id.to_owned(),
            doc: stored.doc,
            precondition: CatalogDocumentPrecondition::Version(stored.version),
            schema: stored.meta.schema,
            expires_ms: stored.meta.expires_ms,
            created_ms: Some(stored.meta.created_ms),
        },),
    )
    .expect("write the corrupt flow fixture through its owner-scoped port");
    assert!(matches!(reply, CatalogStorageReply::Value { .. }));
}

/// The success-path mirror of the refusal test above: when nothing rewrites
/// the flow between park and resume, the recomputed hash matches the pinned
/// one and the resume proceeds exactly as it did before this guard existed.
#[tokio::test]
async fn flows_resume_succeeds_when_the_graph_is_unchanged() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    // This graph contains no inference node. Resuming a parked non-LLM flow
    // must not gain a provider-readiness dependency from the StartRun path.
    let created = flows_create(&config, "gated".to_string(), approval_gated_graph(), false)
        .await
        .unwrap();

    let run = flows_run(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &created.value.id,
        json!({ "x": 1 }),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let thread_id = run.value["thread_id"].as_str().unwrap().to_string();
    let pending: Vec<String> =
        serde_json::from_value(run.value["pending_approvals"].clone()).unwrap();

    let parked_row = flows_get_run(&config, &thread_id).await.unwrap().value;
    assert!(
        parked_row.graph_hash.is_some(),
        "a freshly parked run must pin the graph it parked against"
    );

    let resumed = flows_resume(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &created.value.id,
        &thread_id,
        pending,
        vec![],
    )
    .await
    .expect("resume must succeed when the pinned graph still matches the current one");
    let response_keys: std::collections::BTreeSet<_> = resumed
        .value
        .as_object()
        .expect("resume keeps the established object response")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        response_keys,
        ["output", "pending_approvals", "thread_id"]
            .into_iter()
            .collect(),
        "ResumeRun's internal admission acknowledgement must not change the public RPC shape"
    );
    assert_eq!(resumed.value["pending_approvals"], json!([]));
    assert!(
        !resumed.value["output"]["nodes"]["downstream"]["items"].is_null(),
        "downstream should run once the gate is approved via resume"
    );

    let run_row = flows_get_run(&config, &thread_id).await.unwrap().value;
    assert_eq!(run_row.status, "completed");
    assert!(
        run_row.graph_hash.is_none(),
        "a settled row clears its park-time pin rather than leaving it stale: {run_row:?}"
    );
}

fn profile_dispatch(config: &Config, profile: &str) -> crate::core::runtime::DispatchCtx {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};

    let root = CoreContext::for_test_with_config(DomainSet::full(), config.clone());
    crate::core::runtime::DispatchCtx::new(
        root.derive_with(
            ContextOverlay::new(
                config.clone(),
                DomainSet::full(),
                crate::tools::toolpacks::ToolGroups::default(),
            )
            .profile(profile),
        ),
    )
}

async fn scoped_run_status(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    run_id: &str,
) -> String {
    use tinyflows_bus::catalog::{self, FlowRun, RunIdRequest};

    let storage = crate::flows::catalog_storage::register(ctx, config)
        .await
        .unwrap()
        .storage();
    let proxy = crate::modules::tinyflows::catalog_proxy(config)
        .await
        .unwrap();
    let run: Option<FlowRun> = proxy
        .call(
            catalog::members::GET_RUN,
            (RunIdRequest {
                storage,
                id: run_id.to_owned(),
            },),
        )
        .await
        .unwrap();
    run.expect("run exists in this scoped Catalog").status
}

#[tokio::test]
async fn resume_uses_the_captured_profile_scope_for_identical_run_ids() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let ctx_a = profile_dispatch(&config, "resume-owner-a");
    let ctx_b = profile_dispatch(&config, "resume-owner-b");
    let flow_a = crate::flows::ops::flows_create_with_context(
        &ctx_a,
        &config,
        "owner-a".to_string(),
        approval_gated_graph(),
        false,
    )
    .await
    .unwrap()
    .value;
    let flow_b = crate::flows::ops::flows_create_with_context(
        &ctx_b,
        &config,
        "owner-b".to_string(),
        approval_gated_graph(),
        false,
    )
    .await
    .unwrap()
    .value;
    let run_id = "same-opaque-run-id";

    for (ctx, flow, owner) in [(&ctx_a, &flow_a, "owner-a"), (&ctx_b, &flow_b, "owner-b")] {
        let (_session, mut events) = crate::flows::runs_adapter::start_run(
            ctx,
            &config,
            flow,
            run_id,
            json!({ "owner": owner }),
            serde_json::Map::new(),
            FlowRunTrigger::Rpc,
        )
        .await
        .unwrap();
        let parked = crate::flows::runs_adapter::wait_for_outcome(&mut events, run_id, 0)
            .await
            .unwrap();
        assert_eq!(parked.status, "pending_approval");
    }

    let resumed_a = flows_resume(
        &ctx_a,
        &config,
        &flow_a.id,
        run_id,
        vec!["gate".to_string()],
        vec![],
    )
    .await
    .expect("the resume is resolved in owner A's Catalog and callback scope");
    assert_eq!(
        resumed_a.value["output"]["run"]["trigger"]["owner"],
        json!("owner-a")
    );
    assert_eq!(
        scoped_run_status(&ctx_a, &config, run_id).await,
        "completed"
    );
    assert_eq!(
        scoped_run_status(&ctx_b, &config, run_id).await,
        "pending_approval",
        "resuming the same opaque run id in profile A must not settle profile B's run"
    );

    let resumed_b = flows_resume(
        &ctx_b,
        &config,
        &flow_b.id,
        run_id,
        vec!["gate".to_string()],
        vec![],
    )
    .await
    .expect("owner B retains its independently parked run");
    assert_eq!(
        resumed_b.value["output"]["run"]["trigger"]["owner"],
        json!("owner-b")
    );
    assert_eq!(
        scoped_run_status(&ctx_b, &config, run_id).await,
        "completed"
    );
}

/// Migration safety (T-M1 requirement #4): a `flow_runs` row written before
/// this guard existed reads back with `graph_hash IS NULL`. That must be
/// treated as "unknown — allow, with a warning", never as a hard refusal, so
/// upgrading mid-park can never strand an otherwise-valid in-flight approval
/// — even if the flow's graph was *also* edited in the meantime, since there
/// is nothing recorded to compare it against.
///
/// Multi-thread on purpose: this test calls the synchronous store bridge
/// (`update_parked_run_document_for_test`, `store::update_flow_graph`) between
/// async calls. The bridge blocks the calling thread while the TinyFlows
/// module calls back into the host's document port, and a current-thread
/// runtime has no other thread left to make progress on the work that call
/// depends on (the resumed run's poller task lives on that runtime). The
/// production runtime is multi-thread, where the bridge uses `block_in_place`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn flows_resume_allows_a_legacy_row_with_null_graph_hash() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "gated".to_string(), approval_gated_graph(), false)
        .await
        .unwrap();

    let run = flows_run(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &created.value.id,
        json!({ "x": 1 }),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let thread_id = run.value["thread_id"].as_str().unwrap().to_string();
    let pending: Vec<String> =
        serde_json::from_value(run.value["pending_approvals"].clone()).unwrap();

    // Simulate a row written before the T-M1 migration: still `pending_approval`,
    // but with no graph hash pinned — exactly what `add_column_if_missing`
    // leaves behind for every row that existed before this feature shipped.
    update_parked_run_document_for_test(&config, &thread_id, |document| {
        document
            .as_object_mut()
            .expect("run document is an object")
            .remove("graph_hash");
    });
    let staged = flows_get_run(&config, &thread_id).await.unwrap().value;
    assert!(
        staged.graph_hash.is_none(),
        "fixture must simulate a legacy row with no pin"
    );

    // The flow is ALSO rewritten afterward — a legacy row has nothing to
    // compare against, so this must not matter.
    let mut rewritten = approval_gated_graph();
    rewritten["nodes"][2]["name"] = json!("Downstream (renamed)");
    store::update_flow_graph(
        &config,
        &created.value.id,
        created.value.name.clone(),
        structurally_valid_graph(rewritten),
        created.value.require_approval,
        None,  // enabled_override
        false, // force_disarm_if_automatic — this fixture isn't exercising the
        // manual->automatic disarm path, only the graph swap.
        None,
    )
    .unwrap();

    let resumed = flows_resume(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &created.value.id,
        &thread_id,
        pending,
        vec![],
    )
    .await
    .expect("a legacy row with no graph_hash must still resume (unknown treated as allow)");
    assert_eq!(resumed.value["pending_approvals"], json!([]));
}
