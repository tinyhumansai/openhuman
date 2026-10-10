use super::*;

use crate::memory::scope::MemoryIdentity;
use crate::memory::test_fixtures::{bind_reference, config_in};

#[test]
fn the_policy_mirrors_the_recall_config() {
    let mut recall = MemoryRecallConfig::default();
    let default = policy(&recall);
    assert_eq!(
        default,
        RecallPolicy::default(),
        "the defaults agree with TinyMemory's"
    );

    recall.build_beliefs_every = 0;
    recall.team_limit = 0;
    recall.budget_tokens = 0;
    let tuned = policy(&recall);
    assert_eq!(tuned.build_beliefs_every, None);
    assert_eq!(tuned.team_limit, 0);
    assert_eq!(
        tuned.budget_tokens, 1,
        "a zero budget is clamped, never refused"
    );

    let quiet = log_only(default.clone());
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
    let identity = MemoryIdentity::team_member("acme", "writer").resolve(&config);
    assert!(matches!(
        agent_memory(&config, &identity),
        Err(MemoryError::Off(_))
    ));

    bind_reference(&config);
    let memory = agent_memory(&config, &identity).unwrap();
    assert_eq!(memory.agent_id(), "writer");
    assert_eq!(memory.namespace().to_string(), "team:acme/agent:writer");

    let (named, _) = named_agent_memory(&config, Some("auditor")).unwrap();
    assert_eq!(named.namespace().to_string(), "agent:auditor");
    let (current, resolved) = crate::memory::scope::within_agent("planner", async {
        current_agent_memory(&config).unwrap()
    })
    .await;
    assert_eq!(current.agent_id(), "planner");
    assert_eq!(resolved.agent_id, "planner");
}
