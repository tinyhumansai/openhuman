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
        current().map(|stores| stores.transcripts.destination_key())
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
    assert!(current().is_none() || is_installed());
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
async fn without_a_store_transcripts_are_workspace_files() {
#[cfg(unix)]
    let dir = std::path::Path::new("/nonexistent-openhuman-workspace");
    #[cfg(windows)]
    let dir = std::path::Path::new(r"C:\nonexistent-openhuman-workspace");
    assert_eq!(
        transcripts_or_files("u1", dir).destination_key(),
        // tinyagents keys the file destination on the resolved `session_raw`
        // directory under the workspace (tinyagents#06ad95ea).
        Some(dir.join("session_raw").to_string_lossy().into_owned())
    );
    let provider: Arc<dyn SessionStoreProvider> = Arc::new(InMemorySessionStores::new());
    let expected = provider.for_agent("u1").transcripts.destination_key();
    let found = scope(provider, async {
        transcripts_or_files("u1", dir).destination_key()
    })
    .await;
    assert_eq!(found, expected);
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
async fn desktop_session_keys_and_transcript_roots_are_unchanged() {
    let ws = std::path::Path::new("/ws");
    assert_eq!(current_agent_key_or("orchestrator"), "orchestrator");
    assert_eq!(transcript_root(ws), ws.to_path_buf());
    let (key, root) = under(derived(None, Some("alpha")), || {
        (current_agent_key_or("orchestrator"), transcript_root(ws))
    })
    .await;
    assert_eq!(key, "alpha");
    assert_eq!(root, agent_transcript_root(ws, "alpha"));
}

#[tokio::test]
async fn two_profiles_default_agents_keep_separate_session_keys_and_stores() {
    let ws = std::path::Path::new("/ws-profile");
    let (alice_key, alice_root) = under(derived(Some("alice"), None), || {
        (current_agent_key_or("orchestrator"), transcript_root(ws))
    })
    .await;
    let bob_key = under(derived(Some("bob"), None), || {
        current_agent_key_or("orchestrator")
    })
    .await;
    assert_eq!(alice_key, "alice~default");
    assert_eq!(bob_key, "bob~default");
    // The default agent's transcripts sit at the profile's workspace root.
    assert_eq!(alice_root, ws.to_path_buf());

    let provider: Arc<dyn SessionStoreProvider> = Arc::new(InMemorySessionStores::new());
    let stores_of = |profile: &'static str| {
        let provider = provider.clone();
        crate::core::runtime::CoreContext::scope(
            derived_with_store(Some(profile), None, provider),
            async { current().map(|stores| stores.transcripts.destination_key()) },
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
    release_workspace_defaults();

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

    release_workspace_defaults();

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
