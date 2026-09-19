use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tinyagents_runtime::{
    DriverFailure, DriverOutcome, DriverRequest, PrefixSnapshot, ResumeMode, SessionDriver,
    SessionTerminal, SessionTurnRequest, ToolSnapshot, TranscriptCodec, TranscriptTurnOptions,
    TurnOptions,
};
use tinyagents_session::transcript::{FileTranscriptLocator, TranscriptMeta};
use tinyinference_llm::message::Message;

use super::runtime_session::{
    account_committed_turn_against_goal, holistic_last_turn_usage, reconcile_synthesized_visibility,
};
use super::{OpenHumanSessionFactory, OpenHumanSessionHooks, OpenHumanTranscriptCodec};
use crate::agent::tinyagents::host::OpenHumanRunContext;

struct RecordingDriver(Arc<Mutex<Vec<OpenHumanRunContext>>>);

#[async_trait]
impl SessionDriver<OpenHumanRunContext> for RecordingDriver {
    async fn execute(
        &self,
        request: DriverRequest<OpenHumanRunContext>,
    ) -> Result<DriverOutcome, DriverFailure> {
        self.0
            .lock()
            .unwrap()
            .push(request.run_context.data.clone());
        let mut history = request.history;
        history.push(Message::assistant("done"));
        Ok(DriverOutcome {
            history,
            output: Some("done".into()),
            partial: None,
            interrupted: false,
        })
    }
}

#[test]
fn codec_marks_failed_tool_rows_from_the_explicit_turn_sidecar() {
    let context = OpenHumanRunContext::new();
    context.session_sidecar.lock().unwrap().tool_outcomes.push(
        crate::agent::tinyagents::ToolCallOutcome {
            call_id: "call-1".into(),
            name: "dangerous_tool".into(),
            arguments: serde_json::json!({"path": "secret"}),
            success: false,
            content: "denied".into(),
            duration_ms: 0,
        },
    );
    let rows = OpenHumanTranscriptCodec
        .reconcile(
            &[],
            &[],
            &[Message::tool("call-1", "denied")],
            &TranscriptTurnOptions {
                request_id: Some("request-1".into()),
                thread_id: None,
                stream: false,
                resume: ResumeMode::Never,
                context,
            },
        )
        .unwrap();
    assert!(
        rows[0]
            .tool_failure
            .as_ref()
            .is_some_and(|failure| failure.failed)
    );
}

#[test]
fn codec_attaches_sidecar_usage_and_exact_tool_arguments_to_atomic_append() {
    let context = OpenHumanRunContext::new();
    {
        let mut sidecar = context.session_sidecar.lock().unwrap();
        sidecar.model_calls = 2;
        sidecar.input_tokens = 13;
        sidecar.output_tokens = 8;
        sidecar.cached_input_tokens = 3;
        sidecar.cost_usd = 0.004;
        sidecar.context_window = 128_000;
        sidecar
            .tool_outcomes
            .push(crate::agent::tinyagents::ToolCallOutcome {
                call_id: "call-usage".into(),
                name: "read_file".into(),
                arguments: serde_json::json!({"path": "Cargo.toml"}),
                success: true,
                content: "[package]".into(),
                duration_ms: 7,
            });
    }
    context.append_subagent_usage(crate::agent::tinyagents::host::SubagentUsageEntry {
        task_id: "child-usage".into(),
        agent_id: "researcher".into(),
        usage: crate::agent::harness::subagent_runner::SubagentUsage {
            input_tokens: 5,
            output_tokens: 2,
            cached_input_tokens: 1,
            charged_amount_usd: 0.001,
        },
    });
    let usage = OpenHumanTranscriptCodec
        .turn_usage(&TranscriptTurnOptions {
            request_id: Some("request-usage".into()),
            thread_id: None,
            stream: false,
            resume: ResumeMode::Never,
            context,
        })
        .unwrap()
        .expect("observed sidecar produces transcript usage");

    assert_eq!(usage.usage.input, 18, "direct + completed child input");
    assert_eq!(usage.usage.output, 10, "direct + completed child output");
    assert_eq!(usage.usage.cached_input, 4);
    assert_eq!(usage.usage.context_window, 128_000);
    assert!((usage.usage.cost_usd - 0.005).abs() < f64::EPSILON);
    assert_eq!(usage.iteration, 2);
    assert_eq!(usage.tool_calls.len(), 1);
    assert_eq!(usage.tool_calls[0].id, "call-usage");
    assert_eq!(usage.tool_calls[0].arguments, r#"{"path":"Cargo.toml"}"#);
}

#[test]
fn last_turn_usage_reports_the_same_holistic_totals_as_transcript_billing() {
    let mut sidecar = crate::agent::tinyagents::host::run_context::SessionTurnSidecar {
        input_tokens: 13,
        output_tokens: 8,
        cached_input_tokens: 3,
        cost_usd: 0.004,
        context_window: 128_000,
        ..Default::default()
    };
    sidecar
        .subagents
        .push(crate::agent::tinyagents::host::SubagentUsageEntry {
            task_id: "child-ui".into(),
            agent_id: "researcher".into(),
            usage: crate::agent::harness::subagent_runner::SubagentUsage {
                input_tokens: 5,
                output_tokens: 2,
                cached_input_tokens: 1,
                charged_amount_usd: 0.001,
            },
        });

    let usage = holistic_last_turn_usage(&sidecar);
    assert_eq!(usage.input_tokens, 18);
    assert_eq!(usage.output_tokens, 10);
    assert_eq!(usage.cached_input_tokens, 4);
    assert!((usage.cost_usd - 0.005).abs() < f64::EPSILON);
    assert_eq!(usage.context_window, 128_000);
    assert_eq!(usage.subagents.len(), 1, "detail breakdown is retained");
}

#[test]
fn explicit_hidden_delegate_stays_hidden_across_dynamic_refreshes() {
    let mut visible = ["read_file".to_string()]
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let prior = [
        "delegate_research".to_string(),
        "delegate_calendar".to_string(),
    ]
    .into_iter()
    .collect::<std::collections::HashSet<_>>();
    let refreshed = [
        "delegate_research".to_string(),
        "delegate_drive".to_string(),
    ]
    .into_iter()
    .collect::<std::collections::HashSet<_>>();

    reconcile_synthesized_visibility(&mut visible, &prior, &refreshed, false);

    assert!(
        !visible.contains("delegate_research"),
        "an explicitly hidden existing delegate remains hidden after refresh"
    );
    assert!(
        !visible.contains("delegate_drive"),
        "a new connection cannot bypass an explicit visibility restriction"
    );
    assert!(
        !visible.contains("delegate_calendar"),
        "a revoked connection leaves the surface"
    );
}

#[test]
fn wildcard_delegate_visibility_admits_new_connections_and_removes_revoked_ones() {
    let mut visible = ["read_file".to_string(), "delegate_calendar".to_string()]
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let prior = ["delegate_calendar".to_string()]
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let refreshed = ["delegate_drive".to_string()]
        .into_iter()
        .collect::<std::collections::HashSet<_>>();

    reconcile_synthesized_visibility(&mut visible, &prior, &refreshed, true);

    assert!(visible.contains("delegate_drive"));
    assert!(!visible.contains("delegate_calendar"));
    assert!(visible.contains("read_file"));
}

#[tokio::test]
async fn committed_goal_accounting_uses_direct_and_completed_child_usage() {
    let workspace = tempfile::tempdir().unwrap();
    crate::agent::goals::store::set(
        workspace.path(),
        "receipt-goal",
        "finish migration",
        Some(20),
    )
    .await
    .unwrap();
    let mut sidecar = crate::agent::tinyagents::host::run_context::SessionTurnSidecar {
        input_tokens: 9,
        output_tokens: 4,
        duration: Some(std::time::Duration::from_secs(2)),
        ..Default::default()
    };
    sidecar
        .subagents
        .push(crate::agent::tinyagents::host::SubagentUsageEntry {
            task_id: "child-goal".into(),
            agent_id: "researcher".into(),
            usage: crate::agent::harness::subagent_runner::SubagentUsage {
                input_tokens: 5,
                output_tokens: 3,
                ..Default::default()
            },
        });

    account_committed_turn_against_goal(workspace.path(), Some("receipt-goal"), &sidecar).await;

    let goal = crate::agent::goals::store::get(workspace.path(), "receipt-goal")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        goal.tokens_used, 21,
        "receipt accounts direct + child usage"
    );
    assert_eq!(
        goal.status,
        crate::agent::goals::ThreadGoalStatus::BudgetLimited,
        "receipt-backed accounting advances continuation/budget state"
    );
}

fn meta() -> TranscriptMeta {
    TranscriptMeta {
        agent_name: "adapter-test".into(),
        agent_id: Some("adapter-test".into()),
        agent_type: Some("root".into()),
        dispatcher: "test".into(),
        provider: None,
        model: None,
        created: chrono::Utc::now().to_rfc3339(),
        updated: chrono::Utc::now().to_rfc3339(),
        turn_count: 0,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: None,
        task_id: None,
    }
}

#[tokio::test]
async fn runtime_adapter_passes_host_context_commits_before_finalize_and_emits_one_terminal() {
    let seen_context = Arc::new(Mutex::new(Vec::new()));
    let order = Arc::new(Mutex::new(Vec::new()));
    let terminals = Arc::new(Mutex::new(Vec::new()));
    let hooks = Arc::new(OpenHumanSessionHooks::new(
        |_, _, _| Box::pin(async { Ok(Default::default()) }),
        |_, _, _| Box::pin(async { Ok(Default::default()) }),
        |_, _| Box::pin(async { Ok(()) }),
        {
            let order = Arc::clone(&order);
            move |_| {
                let order = Arc::clone(&order);
                Box::pin(async move {
                    order.lock().unwrap().push("commit");
                    Ok(())
                })
            }
        },
        {
            let order = Arc::clone(&order);
            let terminals = Arc::clone(&terminals);
            move |terminal| {
                let order = Arc::clone(&order);
                let terminals = Arc::clone(&terminals);
                Box::pin(async move {
                    order.lock().unwrap().push("terminal");
                    terminals.lock().unwrap().push(terminal);
                    Ok(())
                })
            }
        },
    ));
    let root = std::env::temp_dir().join(format!(
        "openhuman-session-adapter-{}",
        uuid::Uuid::new_v4()
    ));
    let context = OpenHumanRunContext::new();
    let cancellation = context.cancellation.clone();
    let mut session = OpenHumanSessionFactory::build(
        Arc::new(RecordingDriver(Arc::clone(&seen_context))),
        hooks,
        PrefixSnapshot::default(),
        ToolSnapshot::default(),
        Arc::new(FileTranscriptLocator::new(&root)),
        "adapter-test",
        meta(),
    )
    .unwrap();

    session
        .turn(
            SessionTurnRequest::new(Message::user("hello")),
            TurnOptions {
                request_id: Some("request-1".into()),
                thread_id: Some("thread-1".into()),
                stream: false,
                resume: Default::default(),
                cancellation: cancellation.clone(),
                run_context: context
                    .into_tinyagents(tinyagents_harness::context::RunConfig::new("adapter-turn")),
            },
        )
        .await
        .unwrap();

    assert_eq!(seen_context.lock().unwrap().len(), 1);
    assert_eq!(*order.lock().unwrap(), ["commit", "terminal"]);
    assert!(matches!(
        terminals.lock().unwrap().as_slice(),
        [SessionTerminal::Completed(_)]
    ));
    assert!(
        root.join("session_raw").exists(),
        "commit precedes finalization"
    );
    let _ = std::fs::remove_dir_all(root);
}
