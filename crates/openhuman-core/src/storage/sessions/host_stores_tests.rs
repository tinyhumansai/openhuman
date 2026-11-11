use super::*;
use tinyagents_session::run_ledger::AgentRunListRequest;

fn scratch() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

fn port_scope(scope: crate::storage::Scope) -> tinyagents_storage_kit::Scope {
    tinyagents_storage_kit::Scope::new(scope.as_str()).expect("valid storage scope")
}

#[test]
fn the_calling_tenants_stores_open_on_the_workspace_default() {
    let ws = scratch();
    let stores = stores_for(ws.path()).expect("the operator has a tenant");
    let state = tinyagents_session::turn_state::TurnState::started(
        "thread-s",
        "req-s",
        4,
        "2026-10-09T00:00:00Z",
    );
    stores.turn_states.put(&state).unwrap();
    let again = stores_or_ephemeral(ws.path());
    assert!(
        again.turn_states.get("thread-s").unwrap().is_some(),
        "both resolve the same workspace default"
    );
}

#[test]
fn turn_states_for_the_tenant_and_for_the_operator_share_the_default_agent() {
    let ws = scratch();
    let state = tinyagents_session::turn_state::TurnState::started(
        "thread-w",
        "req-w",
        4,
        "2026-10-09T00:00:00Z",
    );
    turn_states_for(ws.path()).put(&state).unwrap();
    let seen = default_turn_states_for(ws.path())
        .get("thread-w")
        .unwrap()
        .expect("the operator is the default agent");
    assert_eq!(seen.thread_id, "thread-w");
}

#[test]
fn an_attachment_stash_and_artifact_blobs_bind_the_tenant_scope() {
    let ws = scratch();
    assert!(attachment_stash_for(ws.path()).is_some());
    let blobs = artifact_blobs_for(ws.path()).expect("artifact blobs bind");
    let documents = current_documents_for(ws.path());
    assert!(documents.is_some());
    // The blob half is usable: an unknown key reads as absent.
    let found = crate::storage::block_on(async move { Ok(blobs.get("outputs/none").await) });
    assert!(matches!(found, Ok(Ok(None))), "{found:?}");
}

#[tokio::test]
async fn artifact_scope_uses_the_carried_profile_when_ambient_profile_differs() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};

    let ws = scratch();
    let context_for = |profile: &'static str| {
        let overlay = ContextOverlay::new(
            crate::config::Config::default(),
            DomainSet::kernel(),
            crate::tools::toolpacks::ToolGroups::none(),
        )
        .profile(profile);
        CoreContext::for_test(DomainSet::full(), None).derive_with(overlay)
    };
    let artifact_context = DispatchCtx::new(context_for("artifact-owner"));
    let ambient_context = context_for("different-ambient-owner");

    CoreContext::scope(ambient_context, async {
        let scoped = artifact_scoped_for_in(ws.path(), &artifact_context)
            .expect("the explicit artifact owner has a storage scope");
        assert_eq!(
            scoped.scope(),
            &port_scope(crate::storage::scope_for_profile("artifact-owner")),
            "artifacts share the profile scope and follow the carried context"
        );
    })
    .await;
}

#[test]
fn artifact_scope_uses_agent_scope_only_for_unprofiled_named_agents() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};

    let ws = scratch();
    let context_for = |agent: Option<&str>| {
        let mut overlay = ContextOverlay::new(
            crate::config::Config::default(),
            DomainSet::kernel(),
            crate::tools::toolpacks::ToolGroups::none(),
        );
        if let Some(agent) = agent {
            overlay = overlay.session_agent(agent);
        }
        DispatchCtx::new(CoreContext::for_test(DomainSet::full(), None).derive_with(overlay))
    };

    let named_agent = artifact_scoped_for_in(ws.path(), &context_for(Some("writer")))
        .expect("an unprofiled named agent has a local scope");
    assert_eq!(
        named_agent.scope(),
        &port_scope(crate::storage::scope_for_agent("writer"))
    );

    let default_agent = artifact_scoped_for_in(ws.path(), &context_for(None))
        .expect("the default agent has the local scope");
    assert_eq!(default_agent.scope().as_str(), "local");

    let explicitly_named_default = artifact_scoped_for_in(
        ws.path(),
        &context_for(Some(crate::storage::sessions::DEFAULT_AGENT)),
    )
    .expect("an explicitly named agent uses the agent scope");
    assert_eq!(
        explicitly_named_default.scope(),
        &port_scope(crate::storage::scope_for_agent(
            crate::storage::sessions::DEFAULT_AGENT,
        ))
    );
}

#[test]
fn a_run_status_store_is_built_for_the_workspace() {
    let ws = scratch();
    let _status = status_store_for(ws.path()).expect("status store");
}

#[test]
fn the_ledger_serves_reads_from_async_and_sync_callers() {
    let ws = scratch();
    assert!(ledger_for(ws.path()).is_some());
    assert!(require_ledger(ws.path()).is_ok());

    let sync = with_ledger(ws.path(), |ledger| async move {
        ledger
            .list_agent_runs(&AgentRunListRequest::default())
            .await
    })
    .expect("the sync bridge lists an empty ledger");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let asynchronous = runtime
        .block_on(with_ledger_async(ws.path(), |ledger| async move {
            ledger
                .list_agent_runs(&AgentRunListRequest::default())
                .await
        }))
        .expect("the async path lists an empty ledger");
    assert_eq!(
        serde_json::to_value(&sync).unwrap(),
        serde_json::to_value(&asynchronous).unwrap()
    );
}

#[test]
#[cfg(feature = "storage-sqlite")]
fn an_unusable_workspace_degrades_to_an_in_memory_store() {
    // A path under a regular file cannot hold a storage directory, so the
    // default backend cannot open and the provider degrades rather than fails.
    let ws = scratch();
    let file = ws.path().join("not-a-dir");
    std::fs::write(&file, b"x").unwrap();
    let unusable = file.join("workspace");

    let first = fallback_provider(&unusable);
    let second = fallback_provider(&unusable);
    assert_eq!(first.destination_key(), second.destination_key());
    first
        .try_for_agent("degraded")
        .expect("the in-memory fallback opens an agent");
}

#[test]
fn releasing_defaults_lets_the_next_call_open_afresh() {
    let ws = scratch();
    let before = fallback_provider(ws.path());
    let cached = fallback_provider(ws.path());
    assert!(
        Arc::ptr_eq(&before, &cached),
        "a repeat call reuses the provider"
    );
    release_workspace(ws.path());
    let after = fallback_provider(ws.path());
    assert!(
        !Arc::ptr_eq(&before, &after),
        "release drops the cached provider"
    );
}

#[test]
fn releasing_one_workspace_keeps_the_others_open() {
    let (mine, other) = (scratch(), scratch());
    let kept = fallback_provider(other.path());
    let released = fallback_provider(mine.path());
    release_workspace(mine.path());
    assert!(
        Arc::ptr_eq(&kept, &fallback_provider(other.path())),
        "another workspace keeps its provider"
    );
    assert!(
        !Arc::ptr_eq(&released, &fallback_provider(mine.path())),
        "the released workspace opens afresh"
    );
}

#[test]
fn the_workspace_session_stores_are_a_workspace_default_with_nothing_to_recover() {
    let stores = WorkspaceSessionStores;
    assert!(stores.recover().is_ok());
    assert_eq!(
        stores.destination_key().as_deref(),
        Some("workspace-default")
    );
}
