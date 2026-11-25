use super::*;

use crate::memory::test_fixtures::{bind_reference, config_in};

#[test]
fn the_policy_mirrors_the_recall_config() {
    let mut recall = RecallSettings::default();
    let default = policy(&recall);
    assert_eq!(
        default,
        RecallPolicy::default(),
        "the defaults agree with TinyMemory's"
    );

    recall.budget_tokens = 1700;
    recall.learnings_limit = 9;
    recall.brain_limit = 5;
    recall.history_limit = 4;
    recall.team_limit = 2;
    recall.build_beliefs_every = 7;
    let tuned = policy(&recall);
    assert_eq!(
        (
            tuned.budget_tokens,
            tuned.learnings_limit,
            tuned.brain_limit,
            tuned.history_limit,
            tuned.team_limit,
            tuned.build_beliefs_every,
        ),
        (1700, 9, 5, 4, 2, Some(7)),
        "every policy field follows the configured recall setting"
    );

    recall.build_beliefs_every = 0;
    recall.team_limit = 0;
    recall.budget_tokens = 0;
    let zeroed = policy(&recall);
    assert_eq!(zeroed.build_beliefs_every, None);
    assert_eq!(zeroed.team_limit, 0);
    assert_eq!(
        zeroed.budget_tokens, 1,
        "a zero budget is clamped, never refused"
    );

    let quiet = default.log_only();
    assert_eq!(
        (
            quiet.learnings_limit,
            quiet.brain_limit,
            quiet.history_limit,
            quiet.team_limit
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(quiet.budget_tokens, default.budget_tokens);
}

#[tokio::test]
async fn agent_memory_needs_an_engine_and_follows_the_identity() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let identity = tinymemory_tools::layout::identity::Identity::team_member("acme", "writer")
        .resolve(&crate::memory::scope::identity_pins(&config));
    assert!(matches!(
        agent_memory(&config, &identity).await,
        Err(Error::Off(_))
    ));

    bind_reference(&config);
    let memory = agent_memory(&config, &identity).await.unwrap();
    assert_eq!(memory.agent_id(), "writer");
    assert_eq!(memory.namespace().to_string(), "team:acme/agent:writer");

    let (named, _) = named_agent_memory(
        &config,
        &tinymemory_tools::layout::identity::Identity::root(),
        Some("auditor"),
    )
    .await
    .unwrap();
    assert_eq!(named.namespace().to_string(), "agent:auditor");
    let (current, resolved) = current_agent_memory(
        &config,
        &tinymemory_tools::layout::identity::Identity::root().child("planner"),
    )
    .await
    .unwrap();
    assert_eq!(current.agent_id(), "planner");
    assert_eq!(resolved.agent_id, "planner");
}
