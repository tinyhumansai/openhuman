use super::*;
use tinyagents_session::port::InMemorySessionStores;

/// Runs `future` under a derived context whose host overrides carry `provider`
/// as the agent's session store, the explicit per-context seam `installed()`
/// consults ahead of the process-wide provider.
async fn scope<F: std::future::Future>(
    provider: Arc<dyn SessionStoreProvider>,
    future: F,
) -> F::Output {
    crate::core::runtime::CoreContext::scope(derived_with_store(None, None, provider), future).await
}

#[tokio::test]
async fn a_scoped_provider_serves_only_its_own_task() {
    let provider: Arc<dyn SessionStoreProvider> = Arc::new(InMemorySessionStores::new());
    let inside = scope(provider.clone(), async {
        assert!(is_installed());
        for_agent("u1").map(|stores| stores.transcripts.destination_key())
    })
    .await;
    assert_eq!(
        inside,
        Some(provider.for_agent("u1").transcripts.destination_key())
    );
}

#[tokio::test]
async fn work_outside_an_agent_context_uses_the_default_agent() {
    let provider: Arc<dyn SessionStoreProvider> = Arc::new(InMemorySessionStores::new());
    let key = scope(provider.clone(), async {
        current(&crate::core::runtime::DispatchCtx::ambient())
            .map(|stores| stores.transcripts.destination_key())
    })
    .await;
    // No agent context in a unit test: the shared default bucket.
    assert_eq!(
        key,
        Some(
            provider
                .for_agent(DEFAULT_AGENT)
                .transcripts
                .destination_key()
        )
    );
    assert!(current(&crate::core::runtime::DispatchCtx::ambient()).is_none() || is_installed());
}

#[tokio::test]
async fn only_a_store_without_files_replaces_them() {
    assert!(!replaces_files(), "no store: the files are the record");
    let memory: Arc<dyn SessionStoreProvider> = Arc::new(InMemorySessionStores::new());
    assert!(scope(memory, async { replaces_files() }).await);
}

#[tokio::test]
async fn transcripts_come_from_the_store_only_when_one_is_in_effect() {
    assert!(transcripts_for("u1").is_none());
    let provider: Arc<dyn SessionStoreProvider> = Arc::new(InMemorySessionStores::new());
    let expected = provider.for_agent("u1").transcripts.destination_key();
    let found = scope(provider, async {
        transcripts_for("u1").map(|locator| locator.destination_key())
    })
    .await;
    assert_eq!(found, Some(expected));
}

#[tokio::test]
async fn without_a_store_transcripts_come_from_the_workspace_store_not_files() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let locator = transcripts_or_files("u1", dir);
    let key = locator.destination_key();
    assert_ne!(
        key,
        Some(dir.join("session_raw").to_string_lossy().into_owned()),
        "the file layout is no longer a transcript destination"
    );
    // A transcript written through one lookup is visible through the next:
    // both resolve to the same durable workspace store.
    let session = tinyagents_session::transcript::SessionRef::scoped("t1", "u1");
    let meta = tinyagents_session::transcript::TranscriptMeta {
        agent_name: "u1".into(),
        agent_id: Some("u1".into()),
        agent_type: Some("test".into()),
        dispatcher: "native".into(),
        provider: None,
        model: None,
        created: "2026-10-15T00:00:00Z".into(),
        updated: "2026-10-15T00:00:00Z".into(),
        turn_count: 0,
        prefix_message_count: None,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: Some("t1".into()),
        task_id: None,
        session_id: None,
        parent_session_id: None,
    };
    let history = locator
        .open_stem(
            &tinyagents_session::transcript::session_stem(&session),
            meta,
        )
        .expect("open the session's write handle");
    history
        .append(tinyagents_session::transcript::TranscriptMessage::user(
            "hi",
        ))
        .expect("append a message");
    assert!(transcripts_or_files("u1", dir).session_exists(&session));
    assert!(
        dir.join(tinyagents_session::port::STORE_DIR).is_dir(),
        "the workspace store lives under the workspace"
    );
    let provider: Arc<dyn SessionStoreProvider> = Arc::new(InMemorySessionStores::new());
    let expected = provider.for_agent("u1").transcripts.destination_key();
    let found = scope(provider, async {
        transcripts_or_files("u1", dir).destination_key()
    })
    .await;
    assert_eq!(found, expected);
}

#[test]
fn an_unopenable_workspace_degrades_to_a_session_that_is_not_kept() {
    let tmp = tempfile::tempdir().unwrap();
    // A file where the workspace directory should be: the store cannot open.
    let blocker = tmp.path().join("not-a-dir");
    std::fs::write(&blocker, b"x").unwrap();
    let session = tinyagents_session::transcript::SessionRef::scoped("t1", "u1");
    let locator = workspace_transcripts("u1", &blocker);
    assert!(!locator.session_exists(&session));
}

fn derived(profile: Option<&str>, agent: Option<&str>) -> Arc<crate::core::runtime::CoreContext> {
    derived_overlay(profile, agent, None)
}

fn derived_with_store(
    profile: Option<&str>,
    agent: Option<&str>,
    provider: Arc<dyn SessionStoreProvider>,
) -> Arc<crate::core::runtime::CoreContext> {
    derived_overlay(profile, agent, Some(provider))
}

fn derived_overlay(
    profile: Option<&str>,
    agent: Option<&str>,
    provider: Option<Arc<dyn SessionStoreProvider>>,
) -> Arc<crate::core::runtime::CoreContext> {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    let mut overlay = ContextOverlay::new(
        crate::config::Config::default(),
        DomainSet::kernel(),
        crate::tools::toolpacks::ToolGroups::none(),
    );
    if let Some(profile) = profile {
        overlay = overlay.profile(profile);
    }
    if let Some(agent) = agent {
        overlay = overlay.session_agent(agent);
    }
    if let Some(provider) = provider {
        let mut overrides = crate::agent::host_overrides::HostOverrides::default();
        overrides.session_store = Some(provider);
        overlay = overlay.host_overrides(Arc::new(overrides));
    }
    CoreContext::for_test(DomainSet::full(), None).derive_with(overlay)
}

async fn under<T>(ctx: Arc<crate::core::runtime::CoreContext>, f: impl FnOnce() -> T) -> T {
    crate::core::runtime::CoreContext::scope(ctx, async move { f() }).await
}

#[tokio::test]
async fn desktop_session_keys_are_unchanged() {
    assert_eq!(
        current_agent_key_or(
            &crate::core::runtime::DispatchCtx::ambient(),
            "orchestrator"
        ),
        "orchestrator"
    );
    let key = under(derived(None, Some("alpha")), || {
        current_agent_key_or(
            &crate::core::runtime::DispatchCtx::ambient(),
            "orchestrator",
        )
    })
    .await;
    assert_eq!(key, "alpha");
}

#[tokio::test]
async fn two_profiles_default_agents_keep_separate_session_keys_and_stores() {
    let alice_key = under(derived(Some("alice"), None), || {
        current_agent_key_or(
            &crate::core::runtime::DispatchCtx::ambient(),
            "orchestrator",
        )
    })
    .await;
    let bob_key = under(derived(Some("bob"), None), || {
        current_agent_key_or(
            &crate::core::runtime::DispatchCtx::ambient(),
            "orchestrator",
        )
    })
    .await;
    assert_eq!(alice_key, "alice~default");
    assert_eq!(bob_key, "bob~default");

    let provider: Arc<dyn SessionStoreProvider> = Arc::new(InMemorySessionStores::new());
    let stores_of = |profile: &'static str| {
        let provider = provider.clone();
        crate::core::runtime::CoreContext::scope(
            derived_with_store(Some(profile), None, provider),
            async {
                current(&crate::core::runtime::DispatchCtx::ambient())
                    .map(|stores| stores.transcripts.destination_key())
            },
        )
    };
    let (alice, bob) = (stores_of("alice").await, stores_of("bob").await);
    assert_ne!(alice, bob);
    assert_eq!(
        alice,
        Some(
            provider
                .for_agent("alice~default")
                .transcripts
                .destination_key()
        )
    );
}

#[test]
fn the_per_workspace_store_recovers_nothing_at_install_time() {
    // A SaaS core installs it before any profile exists, so `recover` must not
    // resolve (and so create) a workspace database.
    assert!(WorkspaceSessionStores.recover().is_ok());
}

#[test]
fn each_workspace_keeps_its_sessions_in_its_own_database() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let in_first = provider_for_workspace(first.path()).for_agent("agent");
    let in_second = provider_for_workspace(second.path()).for_agent("agent");

    in_first
        .turn_states
        .put(&tinyagents_session::turn_state::TurnState::started(
            "thread",
            "request",
            4,
            "2026-01-01T00:00:00Z",
        ))
        .unwrap();

    assert!(in_first.turn_states.get("thread").unwrap().is_some());
    assert!(
        in_second.turn_states.get("thread").unwrap().is_none(),
        "the other workspace never saw the turn"
    );
    assert!(first.path().join("storage").join("storage.db").is_file());
}

fn started(thread: &str, request: &str) -> tinyagents_session::turn_state::TurnState {
    tinyagents_session::turn_state::TurnState::started(thread, request, 4, "2026-01-01T00:00:00Z")
}

#[tokio::test]
async fn a_threads_turn_state_survives_a_restart_of_the_workspace_store() {
    let dir = tempfile::tempdir().unwrap();
    turn_states_for(dir.path())
        .put(&started("thread-1", "req-1"))
        .unwrap();

    // A restart: every handle and cached default backend is dropped.
    release_workspace(dir.path());

    let resumed = turn_states_for(dir.path())
        .get("thread-1")
        .unwrap()
        .expect("the snapshot is read back from the backend");
    assert_eq!(resumed.request_id, "req-1");
    // The sweep a cold boot runs settles it as interrupted, and it persists.
    let swept = default_turn_states_for(dir.path())
        .mark_all_interrupted("2026-01-01T00:01:00Z")
        .unwrap();
    assert_eq!(swept, 1);
    assert_eq!(
        turn_states_for(dir.path())
            .get("thread-1")
            .unwrap()
            .unwrap()
            .lifecycle,
        tinyagents_session::turn_state::TurnLifecycle::Interrupted
    );
}

#[tokio::test]
async fn a_stashed_attachment_survives_a_restart_of_the_workspace_store() {
    let dir = tempfile::tempdir().unwrap();
    let data_uri = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    let stash = attachment_stash_for(dir.path()).expect("a stash for the default tenant");
    stash.write("att-1", data_uri).await.unwrap();

    release_workspace(dir.path());

    let resumed = attachment_stash_for(dir.path()).expect("a stash after the restart");
    assert_eq!(
        resumed.read_data_uri("att-1").await.unwrap().as_deref(),
        Some(data_uri)
    );
    assert!(resumed
        .read_data_uri("never-stashed")
        .await
        .unwrap()
        .is_none());
}

#[test]
fn agent_key_for_matches_the_session_hosts_resolution() {
    use crate::core::runtime::Tenant;
    let desktop = Tenant::default();
    assert_eq!(agent_key_for(&desktop, "orchestrator"), "orchestrator");
    let embedded = Tenant {
        profile: None,
        agent: Some("a1".into()),
    };
    assert_eq!(agent_key_for(&embedded, "orchestrator"), "a1");
    let profile = Tenant {
        profile: Some("p1".into()),
        agent: None,
    };
    assert_eq!(agent_key_for(&profile, "orchestrator"), "p1~default");
    let profile_agent = Tenant {
        profile: Some("p1".into()),
        agent: Some("a1".into()),
    };
    assert_eq!(
        agent_key_for(&profile_agent, "orchestrator"),
        crate::core::runtime::session_key(&profile_agent)
    );
}
