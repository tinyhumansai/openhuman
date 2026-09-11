use super::*;
use tempfile::TempDir;

fn test_config(dir: &TempDir) -> Config {
    let mut config = Config::default();
    config.workspace_dir = dir.path().to_path_buf();
    config.action_dir = dir.path().join("actions");
    config
}

fn team_err(err: anyhow::Error) -> TeamError {
    err.downcast::<TeamError>().expect("TeamError")
}

#[test]
fn create_team_rejects_duplicate_member_names() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    let err = create_team(
        &config,
        "lead",
        None,
        None,
        &[
            NewMember {
                name: "alice".into(),
                agent_id: None,
            },
            NewMember {
                name: "alice".into(),
                agent_id: None,
            },
        ],
    )
    .unwrap_err();
    assert_eq!(
        team_err(err),
        TeamError::DuplicateMemberName {
            name: "alice".into()
        }
    );
}

#[test]
fn assign_task_rejects_self_unknown_and_cycle() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    let view = create_team(
        &config,
        "lead",
        None,
        None,
        &[NewMember {
            name: "alice".into(),
            agent_id: None,
        }],
    )
    .unwrap();
    let team_id = view.team.id.clone();

    // Unknown dependency.
    let err =
        assign_task(&config, &team_id, "task one", None, None, &["ghost".into()]).unwrap_err();
    assert_eq!(
        team_err(err),
        TeamError::UnknownDependency {
            depends_on: "ghost".into()
        }
    );

    // Seed A, then B depends_on A — fine.
    let a = assign_task(&config, &team_id, "A", None, None, &[]).unwrap();
    let b = assign_task(&config, &team_id, "B", None, None, &[a.id.clone()]).unwrap();

    // No cycle: a fresh candidate depending on B is a plain extension of
    // the existing chain (A -> B -> C).
    let existing = run_ledger::list_agent_team_tasks(&config.workspace_dir, &team_id).unwrap();
    assert!(!has_task_cycle("C", &[b.id.clone()], &existing));

    // Cycle: `has_task_cycle` is exercised directly (rather than through
    // `assign_task`, which only ever creates new tasks and can never make
    // an existing one depend on something newer) with a synthetic fixture
    // list. Using ids distinct from every existing task matters here:
    // `tinyagents_graph::dag::has_cycle` deliberately processes only the
    // *first* declaration of a repeated id and ignores the rest (see its
    // doc comment on `DagIssue::DuplicateNode`), so reusing `a.id` for the
    // new candidate — as this test previously did — is silently ignored
    // by the crate rather than fabricating a cycle, and the assertion
    // that depended on the old host-side Kahn implementation's
    // duplicate-merging behaviour no longer holds. A three-node chain
    // with unique ids (x depends on z, y depends on x, and a new z
    // depending on y) closes a real x -> z -> y -> x loop without ever
    // repeating an id.
    let fixture_existing = vec![
        task_fixture(&team_id, "x", &["z"]),
        task_fixture(&team_id, "y", &["x"]),
    ];
    assert!(has_task_cycle("z", &["y".to_string()], &fixture_existing));
}

fn task_fixture(team_id: &str, id: &str, depends_on: &[&str]) -> AgentTeamTask {
    let now = Utc::now();
    AgentTeamTask {
        id: id.to_string(),
        team_id: team_id.to_string(),
        title: id.to_string(),
        objective: None,
        status: AgentTeamTaskStatus::Todo,
        owner_member_id: None,
        claimed_by_member_id: None,
        claim_token: None,
        depends_on: depends_on.iter().map(|d| d.to_string()).collect(),
        gate_status: "pending".to_string(),
        gate_reason: None,
        evidence: Vec::new(),
        source_run_id: None,
        order_index: 0,
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn self_dependency_is_rejected() {
    // Directly exercise validate_dependencies with a matching id.
    let err = validate_dependencies("task-self", &["task-self".into()], &[]).unwrap_err();
    assert_eq!(
        team_err(err),
        TeamError::SelfDependency {
            task_id: "task-self".into()
        }
    );
}

#[test]
fn message_append_then_list_in_order() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    let view = create_team(
        &config,
        "lead",
        None,
        None,
        &[
            NewMember {
                name: "alice".into(),
                agent_id: None,
            },
            NewMember {
                name: "bob".into(),
                agent_id: None,
            },
        ],
    )
    .unwrap();
    let team_id = view.team.id.clone();
    let alice = view.members[0].id.clone();
    let bob = view.members[1].id.clone();

    message_member(&config, &team_id, Some(&alice), Some(&bob), "first", None).unwrap();
    message_member(&config, &team_id, Some(&bob), Some(&alice), "second", None).unwrap();

    let messages = list_messages(&config, &team_id, None).unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].sequence, 1);
    assert_eq!(messages[1].sequence, 2);
    assert_eq!(messages[0].payload["content"], "first");
    assert_eq!(messages[1].payload["content"], "second");
}

#[test]
fn message_member_lead_origin_and_unknown_from() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    let (team_id, alice) = solo_team(&config, "alice");

    // Lead/user-originated message: from = None → stored as "lead", no
    // member validation on the sender.
    message_member(&config, &team_id, None, Some(&alice), "from the lead", None).unwrap();
    let messages = list_messages(&config, &team_id, None).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].payload["from"], LEAD_SENDER);
    assert_eq!(messages[0].payload["to"], alice);

    // A non-None sender that is not a member is still rejected.
    let err =
        message_member(&config, &team_id, Some("ghost"), Some(&alice), "x", None).unwrap_err();
    assert_eq!(
        team_err(err),
        TeamError::UnknownMember {
            member_id: "ghost".into()
        }
    );
}

/// Create a single-member team and return `(team_id, member_id)`.
fn solo_team(config: &Config, name: &str) -> (String, String) {
    let view = create_team(
        config,
        "lead",
        None,
        None,
        &[NewMember {
            name: name.into(),
            agent_id: None,
        }],
    )
    .unwrap();
    let member_id = view.members[0].id.clone();
    (view.team.id, member_id)
}

#[test]
fn complete_task_gate_passes_and_marks_done() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    let (team_id, alice) = solo_team(&config, "alice");

    let task = assign_task(&config, &team_id, "ship it", None, None, &[]).unwrap();
    let claim = claim_task(&config, &team_id, &task.id, &alice, "tok-1").unwrap();
    assert!(matches!(claim, ClaimOutcome::Claimed(_)));

    let outcome = complete_task(
        &config,
        &team_id,
        &task.id,
        &alice,
        &["https://ci/run/1".to_string()],
        true,
    )
    .unwrap();
    match outcome {
        CompletionOutcome::Completed(done) => {
            assert_eq!(done.status, AgentTeamTaskStatus::Done);
            assert_eq!(done.gate_status, "passed");
            assert_eq!(done.gate_reason, None);
            assert_eq!(done.evidence, vec!["https://ci/run/1".to_string()]);
        }
        other => panic!("expected Completed, got {other:?}"),
    }
}

#[test]
fn complete_task_requires_evidence_then_recovers() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    let (team_id, alice) = solo_team(&config, "alice");

    let task = assign_task(&config, &team_id, "ship it", None, None, &[]).unwrap();
    claim_task(&config, &team_id, &task.id, &alice, "tok-1").unwrap();

    // No evidence + require_evidence → gate fails, task stays in progress.
    let failed = complete_task(&config, &team_id, &task.id, &alice, &[], true).unwrap();
    match failed {
        CompletionOutcome::GateFailed { reasons } => {
            assert!(
                reasons.iter().any(|r| r.contains("evidence")),
                "{reasons:?}"
            );
        }
        other => panic!("expected GateFailed, got {other:?}"),
    }
    let mid = run_ledger::get_agent_team_task(&config.workspace_dir, &task.id)
        .unwrap()
        .unwrap();
    assert_eq!(mid.status, AgentTeamTaskStatus::InProgress);
    assert_eq!(mid.gate_status, "failed");

    // Retry with evidence → passes.
    let ok = complete_task(
        &config,
        &team_id,
        &task.id,
        &alice,
        &["proof".to_string()],
        true,
    )
    .unwrap();
    assert!(matches!(ok, CompletionOutcome::Completed(_)));
}

#[test]
fn complete_task_is_not_double_completable() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    let (team_id, alice) = solo_team(&config, "alice");

    let task = assign_task(&config, &team_id, "ship it", None, None, &[]).unwrap();
    claim_task(&config, &team_id, &task.id, &alice, "tok-1").unwrap();

    let first = complete_task(&config, &team_id, &task.id, &alice, &[], false).unwrap();
    assert!(matches!(first, CompletionOutcome::Completed(_)));

    // A task that is already `done` is no longer in progress, so a second
    // completion is rejected (the `status = 'in_progress'` UPDATE guard makes
    // the CAS airtight even under a concurrent double-complete).
    let second = complete_task(&config, &team_id, &task.id, &alice, &[], false).unwrap();
    assert_eq!(second, CompletionOutcome::NotClaimed);
}

#[test]
fn complete_task_rejects_non_claimant() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    let view = create_team(
        &config,
        "lead",
        None,
        None,
        &[
            NewMember {
                name: "alice".into(),
                agent_id: None,
            },
            NewMember {
                name: "bob".into(),
                agent_id: None,
            },
        ],
    )
    .unwrap();
    let team_id = view.team.id.clone();
    let alice = view.members[0].id.clone();
    let bob = view.members[1].id.clone();

    let task = assign_task(&config, &team_id, "ship it", None, None, &[]).unwrap();
    claim_task(&config, &team_id, &task.id, &alice, "tok-1").unwrap();

    // Bob is a member but not the claimant → NotClaimed.
    let outcome = complete_task(&config, &team_id, &task.id, &bob, &[], false).unwrap();
    assert_eq!(outcome, CompletionOutcome::NotClaimed);

    // Unknown member → typed error (not an outcome).
    let err = complete_task(&config, &team_id, &task.id, "ghost", &[], false).unwrap_err();
    assert_eq!(
        team_err(err),
        TeamError::UnknownMember {
            member_id: "ghost".into()
        }
    );
}

#[test]
fn complete_task_owner_mismatch_fails_gate() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    let view = create_team(
        &config,
        "lead",
        None,
        None,
        &[
            NewMember {
                name: "alice".into(),
                agent_id: None,
            },
            NewMember {
                name: "bob".into(),
                agent_id: None,
            },
        ],
    )
    .unwrap();
    let team_id = view.team.id.clone();
    let alice = view.members[0].id.clone();
    let bob = view.members[1].id.clone();

    // Task owned by bob, but alice claims + tries to complete.
    let task = assign_task(&config, &team_id, "ship it", None, Some(&bob), &[]).unwrap();
    claim_task(&config, &team_id, &task.id, &alice, "tok-1").unwrap();

    let outcome = complete_task(&config, &team_id, &task.id, &alice, &[], false).unwrap();
    match outcome {
        CompletionOutcome::GateFailed { reasons } => {
            assert!(
                reasons.iter().any(|r| r.contains("owned by")),
                "{reasons:?}"
            );
        }
        other => panic!("expected GateFailed, got {other:?}"),
    }
}

#[test]
fn shutdown_member_releases_in_progress_tasks() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    let (team_id, alice) = solo_team(&config, "alice");

    let task = assign_task(&config, &team_id, "ship it", None, None, &[]).unwrap();
    claim_task(&config, &team_id, &task.id, &alice, "tok-1").unwrap();

    let result = shutdown_member(&config, &team_id, &alice).unwrap();
    assert_eq!(result.released_task_ids, vec![task.id.clone()]);
    assert_eq!(result.member.member_status, AgentTeamMemberStatus::Stopped);

    // Task is back to todo and unclaimed → another teammate could claim it.
    let released = run_ledger::get_agent_team_task(&config.workspace_dir, &task.id)
        .unwrap()
        .unwrap();
    assert_eq!(released.status, AgentTeamTaskStatus::Todo);
    assert_eq!(released.claimed_by_member_id, None);
    assert_eq!(released.claim_token, None);
}

#[test]
fn shutdown_member_unknown_errors() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    let (team_id, _alice) = solo_team(&config, "alice");

    let err = shutdown_member(&config, &team_id, "ghost").unwrap_err();
    assert_eq!(
        team_err(err),
        TeamError::UnknownMember {
            member_id: "ghost".into()
        }
    );
}
