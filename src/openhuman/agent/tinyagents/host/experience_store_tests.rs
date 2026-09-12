use super::*;
use crate::openhuman::memory::tool_memory::test_helpers::MockMemory;

fn adapter() -> OpenHumanExperienceStore {
    OpenHumanExperienceStore::new(Arc::new(MockMemory::default()))
}

fn exp(agent: &str, task: &str, outcome: &str, success: bool) -> Experience {
    let e = Experience::new(agent, task, outcome);
    if success {
        e.succeeded()
    } else {
        e
    }
}

#[tokio::test]
async fn empty_store_recalls_nothing() {
    let found = adapter()
        .recall_for("planner", "migrate the schema")
        .await
        .expect("recall must not error on an empty store");
    assert!(found.is_empty());
}

#[tokio::test]
async fn records_and_recalls_a_prior_attempt() {
    let store = adapter();
    store
        .record(&exp(
            "planner",
            "migrate the customer schema",
            "the migration step needed elevated permissions",
            false,
        ))
        .await
        .expect("record");

    let found = store
        .recall_for("planner", "migrate the customer schema")
        .await
        .expect("recall");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].agent_id, "planner");
    assert!(!found[0].success);
    assert!(
        found[0].outcome.contains("elevated permissions"),
        "the recorded prose must survive the round trip: {}",
        found[0].outcome
    );
}

#[tokio::test]
async fn recall_spans_the_shared_store_while_writes_stay_profile_local() {
    // Stand in for a dedicated-profile session: `local` is the profile
    // subtree, `shared` the workspace store a pre-profile build wrote into.
    let local: Arc<dyn Memory> = Arc::new(MockMemory::default());
    let shared: Arc<dyn Memory> = Arc::new(MockMemory::default());

    // Seed the shared store the way a pre-profile build did — unstamped.
    OpenHumanExperienceStore::new(shared.clone())
        .record(&exp(
            "planner",
            "migrate the customer schema",
            "the legacy attempt hit a lock timeout",
            false,
        ))
        .await
        .expect("seed the shared store");

    let store = OpenHumanExperienceStore::with_profile(local.clone(), None)
        .with_shared_recall_memory(Some(shared.clone()));

    // Recall reaches the shared store even though nothing was written to
    // the profile-local one.
    let found = store
        .recall_for("planner", "migrate the customer schema")
        .await
        .expect("recall");
    assert_eq!(
        found.len(),
        1,
        "a profile session must still see pre-profile experience"
    );
    assert!(found[0].outcome.contains("lock timeout"));

    // A new record lands in the profile-local store, not the shared one.
    store
        .record(&exp("planner", "rotate the signing key", "clean run", true))
        .await
        .expect("record");

    // The domain scores rather than filters, so an unrelated query still
    // returns the seeded row — assert on which task each store *holds*,
    // not on the result count.
    let holds_new_task = |found: &[Experience]| {
        found
            .iter()
            .any(|e| e.task.contains("rotate the signing key"))
    };

    let shared_only = OpenHumanExperienceStore::new(shared)
        .recall_for("planner", "rotate the signing key")
        .await
        .expect("recall from the shared store");
    assert!(
        !holds_new_task(&shared_only),
        "writes must not fan out into the shared store, got: {shared_only:?}"
    );

    let local_only = OpenHumanExperienceStore::new(local)
        .recall_for("planner", "rotate the signing key")
        .await
        .expect("recall from the profile-local store");
    assert!(
        holds_new_task(&local_only),
        "the profile-local store is the write target, got: {local_only:?}"
    );
}

#[tokio::test]
async fn recall_excludes_another_agents_attempt() {
    // The domain only score-boosts an agent match, so without the adapter's
    // post-filter this would return the writer's record too.
    let store = adapter();
    store
        .record(&exp(
            "planner",
            "migrate the customer schema",
            "worked",
            true,
        ))
        .await
        .expect("record");
    store
        .record(&exp(
            "writer",
            "migrate the customer schema",
            "worked",
            true,
        ))
        .await
        .expect("record");

    let found = store
        .recall_for("planner", "migrate the customer schema")
        .await
        .expect("recall");
    assert_eq!(found.len(), 1, "only the planner's attempt is in scope");
    assert_eq!(found[0].agent_id, "planner");
}

#[tokio::test]
async fn an_empty_outcome_is_recorded_rather_than_erroring() {
    // The store rejects a blank `lesson`; the trait documents a blank
    // `outcome` as normal. A synthesized lesson reconciles the two.
    let store = adapter();
    store
        .record(&exp("planner", "deploy the service", "", true))
        .await
        .expect("a blank outcome must not be reported as a lost record");

    let found = store
        .recall_for("planner", "deploy the service")
        .await
        .expect("recall");
    assert_eq!(found.len(), 1);
    assert!(found[0].success);
    assert!(!found[0].outcome.trim().is_empty());
}

#[tokio::test]
async fn unrecallable_records_are_dropped_without_error() {
    let store = adapter();
    store
        .record(&exp("", "migrate", "ok", true))
        .await
        .expect("record must not fail");
    store
        .record(&exp("planner", "   ", "ok", true))
        .await
        .expect("record must not fail");
    assert!(store
        .recall_for("planner", "migrate")
        .await
        .expect("recall")
        .is_empty());
}

#[tokio::test]
async fn recall_is_bounded_by_the_host_not_the_runtime() {
    let store = adapter().with_max_hits(0);
    store
        .record(&exp("planner", "deploy the service", "ok", true))
        .await
        .expect("record");
    assert!(store
        .recall_for("planner", "deploy the service")
        .await
        .expect("recall")
        .is_empty());
}

#[tokio::test]
async fn another_agents_records_cannot_crowd_out_this_agents_attempts() {
    // The domain scores an agent match rather than filtering on it, so a
    // busier agent's records rank alongside this one's. With truncation
    // before the agent filter, they could fill every slot and this recall
    // would return nothing even though matching attempts exist.
    let store = adapter().with_max_hits(2);
    for i in 0..8 {
        store
            .record(&exp("writer", &format!("ship release {i}"), "ok", true))
            .await
            .expect("record");
    }
    store
        .record(&exp(
            "planner",
            "ship release 9",
            "planner's own attempt",
            true,
        ))
        .await
        .expect("record");

    let found = store
        .recall_for("planner", "ship release")
        .await
        .expect("recall");

    assert!(
        !found.is_empty(),
        "the planner's attempt must survive a store dominated by another agent"
    );
    assert!(
        found.iter().all(|e| e.agent_id == "planner"),
        "only this agent's attempts may be returned"
    );
    assert!(found.len() <= 2, "max_hits still bounds the result");
}

#[tokio::test]
async fn secrets_are_redacted_before_they_reach_the_store() {
    let store = adapter();
    store
        .record(&exp(
            "planner",
            "call the deployment endpoint",
            "failed until we set token=hunter2supersecret",
            false,
        ))
        .await
        .expect("record");

    let found = store
        .recall_for("planner", "call the deployment endpoint")
        .await
        .expect("recall");
    assert_eq!(found.len(), 1);
    assert!(
        !found[0].outcome.contains("hunter2supersecret"),
        "the domain's redaction guard must not be bypassed: {}",
        found[0].outcome
    );
    // Case-insensitive on purpose. More than one scrubber can run on this
    // path — `agent::experience::redact_text` writes `token=[redacted]`,
    // while the memory store's own safety pass replaces the whole
    // `key=value` with `[REDACTED]` — and which one lands first is not this
    // adapter's contract. What *is* the contract is the assertion above:
    // the secret must not survive. This second assertion only pins that
    // some redaction visibly happened rather than the prose being silently
    // dropped, so it must not break when a stronger scrubber wins.
    assert!(
        found[0].outcome.to_ascii_lowercase().contains("[redacted]"),
        "a redaction marker must survive into the recalled prose: {}",
        found[0].outcome
    );
}

#[test]
fn writes_stay_inside_the_procedural_namespace() {
    // The whole point of the trait is that procedural experience does not
    // leak into the user's declarative memory. Pin the namespace constant
    // this adapter is confined to.
    assert_eq!(
        crate::openhuman::agent::experience::AGENT_EXPERIENCE_NAMESPACE,
        "agent_experience"
    );
}

#[test]
fn partial_outcomes_read_as_unsuccessful() {
    // OpenHuman's ternary outcome has no boolean equivalent; a partial run
    // must not round up to a success.
    let hit = ExperienceHit {
        experience: AgentExperience {
            id: "exp_test".into(),
            created_at_ms: 1,
            updated_at_ms: 1,
            source: ExperienceSource::ToolLoop,
            agent_id: Some("planner".into()),
            entrypoint: None,
            profile_id: None,
            task_fingerprint: "fp".into(),
            task_summary: "migrate the schema".into(),
            tools_used: vec![],
            tool_sequence: vec![],
            outcome: ExperienceOutcome::Partial,
            error_class: None,
            lesson: "recovered after the first tool failed".into(),
            reuse_hint: "switch strategy".into(),
            avoid_hint: Some("do not repeat the failed call".into()),
            confidence: 0.62,
            tags: vec![],
            payload_hash: None,
            dismissed: false,
        },
        score: 1.0,
        match_reasons: vec![],
    };

    let mapped = to_crate(&hit);
    assert!(!mapped.success, "partial is not a success");
    assert!(mapped.outcome.contains("partially succeeded"));
    assert!(mapped.outcome.contains("recovered after the first tool"));
    assert!(mapped.outcome.contains("do not repeat the failed call"));
}

#[test]
fn profile_ids_partition_the_storage_key() {
    let memory: Arc<dyn Memory> = Arc::new(MockMemory::default());
    let none = OpenHumanExperienceStore::with_profile(memory.clone(), None);
    let alice = OpenHumanExperienceStore::with_profile(memory.clone(), Some("alice".to_string()));
    let blank = OpenHumanExperienceStore::with_profile(memory, Some("   ".to_string()));

    let e = exp("planner", "migrate the schema", "ok", true);
    let none_id = none.to_domain(&e).id;
    let alice_id = alice.to_domain(&e).id;
    // A blank profile id must normalize to the profile-less partition, not
    // create a third unreachable one.
    assert_eq!(none_id, blank.to_domain(&e).id);
    assert_ne!(none_id, alice_id);
    assert_eq!(alice.to_domain(&e).profile_id.as_deref(), Some("alice"));
    assert!(none.to_domain(&e).profile_id.is_none());
}

#[test]
fn long_prose_is_truncated_by_characters_not_bytes() {
    let long = "é".repeat(MAX_SUMMARY_CHARS + 50);
    let record = adapter().to_domain(&exp("planner", &long, "ok", true));
    assert_eq!(record.task_summary.chars().count(), MAX_SUMMARY_CHARS);
}

#[test]
fn recorded_rows_carry_no_fabricated_tool_trace() {
    // A fake tool sequence would corrupt `score_experience`'s tool-overlap
    // term for every later query.
    let record = adapter().to_domain(&exp("planner", "deploy", "ok", true));
    assert!(record.tool_sequence.is_empty());
    assert!(record.tools_used.is_empty());
    assert_eq!(record.tags, vec![ADAPTER_TAG.to_string()]);
    assert_eq!(record.source, ExperienceSource::ToolLoop);
}
