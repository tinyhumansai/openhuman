use super::*;

#[tokio::test]
async fn build_session_agent_injects_default_profile_soul_into_prompt() {
    // Building a session agent constructs a memory store, which reaches
    // the embedding seam; before the extraction this needed no setup.
    use crate::openhuman::agent::context::prompt::LearnedContextData;
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let home = config.workspace_dir.join("personalities").join("default");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("SOUL.md"),
        "I am the user-edited Default profile identity.",
    )
    .unwrap();

    let profile = crate::openhuman::agent::profiles::store::built_in_default_profile();
    let agent = Agent::build_session_agent_inner(
        &config,
        "orchestrator",
        None,
        None,
        false,
        Some(&profile),
    )
    .expect("build default-profile session");

    let prompt = agent
        .build_system_prompt(LearnedContextData::default())
        .expect("build_system_prompt");
    assert!(
        prompt.contains("I am the user-edited Default profile identity."),
        "the live Default profile prompt must include personalities/default/SOUL.md"
    );
}

#[tokio::test]
async fn build_session_agent_profile_less_prompt_has_no_personality_soul() {
    // Building a session agent constructs a memory store, which reaches
    // the embedding seam; before the extraction this needed no setup.
    use crate::openhuman::agent::context::prompt::LearnedContextData;
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    // A personalities/alice/SOUL.md exists on disk, but a profile-less session
    // must never pull it — the prompt stays byte-identical to today.
    let home = config.workspace_dir.join("personalities").join("alice");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("SOUL.md"), "I am Alice, a meticulous archivist.").unwrap();

    let agent = Agent::build_session_agent_inner(&config, "orchestrator", None, None, false, None)
        .expect("build_session_agent_inner without a profile should succeed");

    let prompt = agent
        .build_system_prompt(LearnedContextData::default())
        .expect("build_system_prompt");
    assert!(
        !prompt.contains("I am Alice, a meticulous archivist."),
        "a profile-less session must not inject any profile SOUL.md"
    );
}

#[tokio::test]
async fn from_config_for_agent_still_errors_for_a_genuinely_unknown_id() {
    // Building a session agent constructs a memory store, which reaches
    // the embedding seam; before the extraction this needed no setup.
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);

    // No harness definition AND no config.agent_registry entry for this id —
    // the factory must still hard-error rather than silently building an
    // unfiltered/legacy agent.
    //
    // Note: `Agent` intentionally has no `Debug` impl (it holds `Box<dyn
    // Tool>` / provider trait objects), so this must use `match` +
    // `.is_err()` rather than `.expect_err()`, which requires `T: Debug`.
    let result = Agent::from_config_for_agent(&config, "totally_unknown_agent_id_b38");
    assert!(
        result.is_err(),
        "an id with no harness definition and no custom entry must error"
    );
    let err = result.err().unwrap();
    assert!(
        err.to_string().contains("totally_unknown_agent_id_b38"),
        "error should name the unresolved agent id: {err}"
    );
}
