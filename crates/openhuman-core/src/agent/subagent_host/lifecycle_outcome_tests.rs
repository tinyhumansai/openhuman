use super::*;

#[test]
fn paused_outcomes_preserve_answer_choices_and_worker_resume_options() {
    let definitions =
        crate::agent_registry::host_definition::AgentDefinitionRegistry::builtins_only();
    let definition = definitions.get("orchestrator").unwrap();
    let options = SubagentRunOptions {
        worker_thread_id: Some("worker-thread".into()),
        skill_filter_override: Some("docs".into()),
        model_override: Some("worker-model".into()),
        ..SubagentRunOptions::default()
    };
    let outcome = SubagentRunOutcome {
        task_id: "paused-task".into(),
        agent_id: definition.id.clone(),
        output: "partial work".into(),
        iterations: 2,
        failure_cause: None,
        elapsed: std::time::Duration::from_millis(25),
        mode: crate::agent::subagent_host::SubagentMode::Typed,
        status: SubagentRunStatus::AwaitingUser {
            question: "Which version?".into(),
            options: Some(vec!["stable".into(), "preview".into()]),
            checkpoint: None,
        },
        final_history: Vec::new(),
        usage: SubagentUsage::default(),
        artifact_paths: vec!["report.md".into()],
        persistence_disposition:
            tinyagents_orchestration::subagent::SubagentPersistenceDisposition::PauseCommitted,
    };

    let neutral = host_outcome_to_neutral(outcome, definition, &options);
    assert_eq!(neutral.worker_thread_id.as_deref(), Some("worker-thread"));
    assert_eq!(neutral.model_override.as_deref(), Some("worker-model"));
    assert_eq!(
        neutral.extensions["options"],
        serde_json::json!(["stable", "preview"])
    );
    assert_eq!(
        neutral.extensions["artifact_paths"],
        serde_json::json!(["report.md"])
    );
    assert_eq!(neutral.extensions["elapsed_ms"], 25);
    let SubagentOutcomeKind::AwaitingInput(pause) = neutral.status else {
        panic!("a paused host outcome must remain resumable");
    };
    assert_eq!(pause.reason, "Which version?");
    assert_eq!(pause.resume.metadata["worker_thread_id"], "worker-thread");
    assert_eq!(pause.resume.metadata["skill_filter_override"], "docs");
    assert_eq!(pause.resume.metadata["model_override"], "worker-model");
}

#[test]
fn host_outcome_terminal_record_round_trips_through_json_bytes() {
    let definitions =
        crate::agent_registry::host_definition::AgentDefinitionRegistry::builtins_only();
    let definition = definitions.get("orchestrator").unwrap();
    let options = SubagentRunOptions {
        worker_thread_id: Some("worker-thread".into()),
        model_override: Some("worker-model".into()),
        ..SubagentRunOptions::default()
    };
    let outcome = SubagentRunOutcome {
        task_id: "task-123".into(),
        agent_id: definition.id.clone(),
        output: "durable".into(),
        iterations: 2,
        failure_cause: None,
        elapsed: std::time::Duration::from_millis(25),
        mode: crate::agent::subagent_host::SubagentMode::Typed,
        status: SubagentRunStatus::Completed,
        final_history: Vec::new(),
        usage: SubagentUsage::default(),
        artifact_paths: vec![],
        persistence_disposition:
            tinyagents_orchestration::subagent::SubagentPersistenceDisposition::TerminalInserted,
    };

    let neutral = host_outcome_to_neutral(outcome, definition, &options);
    assert_eq!(neutral.extensions["agent_id"], definition.id);
    assert_eq!(neutral.extensions["iterations"], 2);
    assert!(!neutral.extensions.contains_key("task_id"));
    assert!(!neutral.extensions.contains_key("tier"));
    assert!(!neutral.extensions.contains_key("worker_thread_id"));
    assert!(!neutral.extensions.contains_key("model_override"));
    let encoded = serde_json::to_vec(&neutral).unwrap();
    let decoded: tinyagents_orchestration::subagent::SubagentOutcome =
        serde_json::from_slice(&encoded)
            .expect("terminal outcomes written as bytes must deserialize");
    assert_eq!(decoded.task_id, "task-123");
    assert_eq!(decoded.worker_thread_id.as_deref(), Some("worker-thread"));
    assert_eq!(decoded.model_override.as_deref(), Some("worker-model"));
    assert_eq!(
        decoded.tier.as_deref(),
        Some(definition.agent_tier.as_str())
    );
}

#[test]
fn classified_stop_cause_survives_the_neutral_subagent_projection() {
    let definitions =
        crate::agent_registry::host_definition::AgentDefinitionRegistry::builtins_only();
    let definition = definitions.get("orchestrator").unwrap();
    let cause = tinyagents_harness::no_progress::ClassifiedStopCause {
        class: "permission".into(),
        operation: "shell".into(),
        summary: "permission blocked shell".into(),
    };
    let outcome = SubagentRunOutcome {
        task_id: "stopped-task".into(),
        agent_id: definition.id.clone(),
        output: "partial progress".into(),
        iterations: 1,
        failure_cause: Some(cause),
        elapsed: std::time::Duration::ZERO,
        mode: crate::agent::subagent_host::SubagentMode::Typed,
        status: SubagentRunStatus::Incomplete {
            reason: "permission blocked shell".into(),
            failure_cause: None,
        },
        final_history: Vec::new(),
        usage: SubagentUsage::default(),
        artifact_paths: Vec::new(),
        persistence_disposition:
            tinyagents_orchestration::subagent::SubagentPersistenceDisposition::TerminalInserted,
    };

    let neutral = host_outcome_to_neutral(outcome, definition, &SubagentRunOptions::default());
    assert_eq!(
        neutral.extensions["failure_cause"],
        serde_json::json!({
            "class": "permission",
            "operation": "shell",
        })
    );
}

#[test]
fn classified_stop_extension_round_trips_without_the_private_summary() {
    let cause = tinyagents_harness::no_progress::ClassifiedStopCause {
        class: "uncertain_side_effect".into(),
        operation: "web_answer_tool".into(),
        summary: "private URL https://example.com/private?q=secret".into(),
    };
    let extension = encode_classified_stop_cause(&cause);
    let mut neutral = SubagentOutcome::cancelled("task");
    neutral
        .extensions
        .insert(CLASSIFIED_STOP_CAUSE_EXTENSION.into(), extension);

    let serialized = serde_json::to_string(&neutral).expect("neutral outcome serializes");
    assert!(!serialized.contains("example.com"));
    assert!(!serialized.contains("private URL"));
    let reloaded: SubagentOutcome =
        serde_json::from_str(&serialized).expect("neutral outcome reloads");
    let reloaded_extension = reloaded
        .extensions
        .get(CLASSIFIED_STOP_CAUSE_EXTENSION)
        .expect("namespaced stop cause extension");
    let restored = decode_classified_stop_cause(reloaded_extension, "persisted reason")
        .expect("typed cause reloads");

    assert_eq!(restored.class, "uncertain_side_effect");
    assert_eq!(restored.operation, "web_answer_tool");
    assert_eq!(restored.summary, "persisted reason");
}

#[test]
fn host_incomplete_status_round_trips_with_a_sanitized_stop_cause() {
    let definitions =
        crate::agent_registry::host_definition::AgentDefinitionRegistry::builtins_only();
    let definition = definitions.get("orchestrator").unwrap();
    let reason = "stopped before finishing; reconcile external state";
    let cause = tinyagents_harness::no_progress::ClassifiedStopCause {
        class: "uncertain_side_effect".into(),
        operation: "web_answer_tool".into(),
        summary: "private URL https://example.com/private?q=secret".into(),
    };
    let outcome = SubagentRunOutcome {
        task_id: "task".into(),
        agent_id: definition.id.clone(),
        output: "partial".into(),
        iterations: 2,
        failure_cause: Some(cause.clone()),
        elapsed: std::time::Duration::from_millis(10),
        mode: crate::agent::subagent_host::SubagentMode::Typed,
        status: SubagentRunStatus::Incomplete {
            reason: reason.into(),
            failure_cause: Some(cause),
        },
        final_history: Vec::new(),
        usage: SubagentUsage::default(),
        artifact_paths: Vec::new(),
        persistence_disposition:
            tinyagents_orchestration::subagent::SubagentPersistenceDisposition::TerminalInserted,
    };

    let neutral = host_outcome_to_neutral(outcome, definition, &SubagentRunOptions::default());
    let serialized = serde_json::to_string(&neutral).expect("neutral outcome serializes");
    assert!(!serialized.contains("example.com"));
    let reloaded: SubagentOutcome =
        serde_json::from_str(&serialized).expect("neutral outcome reloads");
    let SubagentOutcomeKind::Incomplete(incomplete) = reloaded.status else {
        panic!("incomplete status survives neutral persistence");
    };
    let restored = incomplete_run_status(incomplete.reason, &reloaded.extensions);
    let SubagentRunStatus::Incomplete {
        reason: restored_reason,
        failure_cause: Some(cause),
    } = restored
    else {
        panic!("typed stop cause survives neutral persistence");
    };

    assert_eq!(restored_reason, reason);
    assert_eq!(cause.class, "uncertain_side_effect");
    assert_eq!(cause.operation, "web_answer_tool");
    assert_eq!(cause.summary, reason);
}
