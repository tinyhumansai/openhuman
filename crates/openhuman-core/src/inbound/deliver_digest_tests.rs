//! Mixed digests: each conversation key runs oldest first, inside the
//! delivery budget.

use super::*;

fn mention_at(id: &str, key: &str, received_at_ms: u64) -> InboundEvent {
    events::external(
        "x:mentions",
        key,
        id,
        &format!("hello {id}"),
        json!({ "author": "@a" }),
        received_at_ms,
    )
    .unwrap()
}

#[tokio::test]
async fn a_mixed_digest_runs_each_conversation_key_oldest_first() {
    let triage = ScriptedTriage::new([decision(TriageAction::Acknowledge)]);
    let agent = ScriptedAgent::new([Reply::Text("a"), Reply::Text("b")]);
    let results = memory_results();
    let response = pinned_deliverer(&triage, &agent, &results)
        .deliver(turn(
            vec![
                mention_at("z-old", "mentions", 1),
                mention_at("a-new", "mentions", 9),
                mention_at("d1", "dms", 3),
                trigger_event("u1"),
            ],
            Lane::Digest,
            1,
        ))
        .await;
    assert_eq!(response.status, DeliverStatus::Accepted);
    let mut keys: Vec<String> = agent.runs().into_iter().map(|run| run.key).collect();
    keys.sort();
    assert_eq!(keys, ["dms", "mentions"]);
    let mentions = results
        .get("turn-1:x:mentions:mentions:z-old..a-new")
        .unwrap()
        .unwrap();
    assert_eq!(mentions.result.event_ids, ["z-old", "a-new"]);
    assert!(results
        .get("turn-1:x:mentions:dms:d1..d1")
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn the_rest_of_a_mixed_digest_waits_when_the_pinned_parts_spent_the_budget() {
    // These pin the delivery budget, so the budget must be spent on the agent run
    // alone: screening an event asks TinySecurity, a module call that counts
    // against the same budget and would otherwise decide these tests.
    let _security = crate::security::switch::off_for_this_thread();
    let triage = ScriptedTriage::new([decision(TriageAction::Acknowledge)]);
    let agent = ScriptedAgent::new([Reply::Text("draft"), Reply::Text("late")]);
    let results = memory_results();
    let response = pinned_deliverer(&triage, &agent, &results)
        .with_budget(Duration::ZERO)
        .deliver(turn(
            vec![
                mention("t1"),
                mention_at("d1", "dms", 3),
                trigger_event("u1"),
            ],
            Lane::Digest,
            1,
        ))
        .await;
    assert!(matches!(response.status, DeliverStatus::Retry { .. }));
    assert_eq!(
        agent.runs().len(),
        1,
        "a second part started with no budget"
    );
    assert_eq!(triage.runs(), 0, "triage started with no budget left");
}
