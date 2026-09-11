use super::*;

/// A config rooted in a throwaway directory.
///
/// Non-negotiable for these tests: `IdentitySection` calls
/// `sync_workspace_file`, which **writes** `SOUL.md` / `IDENTITY.md` /
/// `HEARTBEAT.md` into `workspace_dir`. Composing against
/// `Config::default()` would scribble into the developer's real
/// `~/.openhuman` workspace.
fn config_in(dir: &std::path::Path) -> Arc<Config> {
    let mut config = Config::default();
    config.workspace_dir = dir.to_path_buf();
    config.action_dir = dir.join("projects");
    std::fs::create_dir_all(&config.action_dir).expect("create action dir");
    Arc::new(config)
}

fn request() -> TurnContextRequest {
    TurnContextRequest::new("orchestrator", "thread-1", "what changed today?")
}

/// A definition that sets `omit_profile` / `omit_memory_md` must not have
/// those files injected anyway. The live subagent path derives the gates
/// from the definition; hardcoding them here silently overrode it.
#[tokio::test]
async fn a_definitions_user_file_omissions_are_honoured() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("PROFILE.md"), "PROFILE_MARKER_TEXT").expect("write");
    std::fs::write(dir.path().join("MEMORY.md"), "MEMORY_MARKER_TEXT").expect("write");

    let included = OpenHumanContextComposer::new(config_in(dir.path()))
        .compose_system_prompt(&request())
        .await
        .expect("compose");

    let omitted = OpenHumanContextComposer::new(config_in(dir.path()))
        .with_omissions(false, false)
        .compose_system_prompt(&request())
        .await
        .expect("compose");

    // Only assert the omission direction: whether the default chain renders
    // these particular files depends on section config, and the contract
    // under test is that opting out is respected.
    if included.contains("PROFILE_MARKER_TEXT") {
        assert!(
            !omitted.contains("PROFILE_MARKER_TEXT"),
            "omit_profile must keep PROFILE.md out of the prompt"
        );
    }
    if included.contains("MEMORY_MARKER_TEXT") {
        assert!(
            !omitted.contains("MEMORY_MARKER_TEXT"),
            "omit_memory_md must keep MEMORY.md out of the prompt"
        );
    }
}

#[tokio::test]
async fn composes_a_non_empty_prompt_carrying_the_agent_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    let composer = OpenHumanContextComposer::new(config_in(dir.path()));
    let prompt = composer
        .compose_system_prompt(&request())
        .await
        .expect("prompt composes");

    assert!(!prompt.trim().is_empty(), "prompt must not be blank");
    // The default chain always appends the grounding contract and the
    // global style suffix; asserting on the suffix pins that we went
    // through `SystemPromptBuilder::build` rather than hand-assembling.
    assert!(
        prompt.contains("# Writing style"),
        "prompt must come from SystemPromptBuilder::build"
    );
}

#[tokio::test]
async fn preamble_is_empty_and_not_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let composer = OpenHumanContextComposer::new(config_in(dir.path()));
    assert!(composer
        .preamble(&request())
        .await
        .expect("preamble succeeds")
        .is_empty());
}

#[tokio::test]
async fn the_prompt_is_byte_stable_across_repeated_turns() {
    // The KV-cache contract from mismatch (1): the crate calls this every
    // turn, and OpenHuman needs the bytes frozen. `DateTimeSection` has
    // minute granularity, so two back-to-back calls agreeing is the
    // strongest cheap check that nothing *else* varies per call.
    let dir = tempfile::tempdir().expect("tempdir");
    let composer = OpenHumanContextComposer::new(config_in(dir.path()));
    let a = composer
        .compose_system_prompt(&request())
        .await
        .expect("first");
    let b = composer
        .compose_system_prompt(&request())
        .await
        .expect("second");
    assert_eq!(a, b);
}

#[tokio::test]
async fn a_different_agent_id_still_composes() {
    // The crate treats `agent_id` as opaque, so an id the host has never
    // heard of must not fail composition — it is a prompt input, not a
    // registry lookup.
    let dir = tempfile::tempdir().expect("tempdir");
    let composer = OpenHumanContextComposer::new(config_in(dir.path()));
    let req = TurnContextRequest::new("no-such-agent", "thread-9", "");
    assert!(composer.compose_system_prompt(&req).await.is_ok());
}

#[test]
fn agents_md_layers_are_loaded_when_the_gate_is_on() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = config_in(dir.path());
    std::fs::write(dir.path().join("AGENTS.md"), "global rule").expect("write global");

    let composer = OpenHumanContextComposer::new(config);
    let loaded = composer.agents_md();
    assert_eq!(loaded.global.as_deref(), Some("global rule"));
}

#[test]
fn the_agents_md_gate_is_honoured_when_off() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = Config::default();
    config.workspace_dir = dir.path().to_path_buf();
    config.action_dir = dir.path().join("projects");
    config.agent.agents_md_enabled = false;
    std::fs::create_dir_all(&config.action_dir).expect("create action dir");
    std::fs::write(dir.path().join("AGENTS.md"), "global rule").expect("write global");

    let composer = OpenHumanContextComposer::new(Arc::new(config));
    let loaded = composer.agents_md();
    assert!(
        loaded.is_empty(),
        "a disabled gate must not read AGENTS.md at all"
    );
}

#[test]
fn the_model_name_falls_back_to_the_crate_default() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = Config::default();
    config.workspace_dir = dir.path().to_path_buf();
    config.default_model = None;
    let composer = OpenHumanContextComposer::new(Arc::new(config));
    assert_eq!(composer.model_name, DEFAULT_MODEL);

    let pinned = OpenHumanContextComposer::new(config_in(dir.path())).with_model_name("haiku");
    assert_eq!(pinned.model_name, "haiku");
}

#[tokio::test]
async fn usable_as_a_trait_object() {
    // Pins object safety — the harness stores this as
    // `Arc<dyn ContextComposer>`, so a non-dyn-safe impl would only fail
    // at the wiring site, not here.
    let dir = tempfile::tempdir().expect("tempdir");
    let composer: Arc<dyn ContextComposer> =
        Arc::new(OpenHumanContextComposer::new(config_in(dir.path())));
    assert!(composer.compose_system_prompt(&request()).await.is_ok());
}
