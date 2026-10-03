use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tinyagents_runtime::{
    DriverFailure, DriverOutcome, DriverRequest, PrefixSnapshot, ResumeMode, SessionDriver,
    SessionTerminal, SessionTurnRequest, ToolSnapshot, TranscriptCodec, TranscriptTurnOptions,
    TurnOptions,
};
use tinyagents_session::transcript::{FileTranscriptLocator, TranscriptMeta};
use tinyinference_llm::message::Message;

use super::announcement_notes::{
    integration_announcement_note, mcp_announcement_note, skill_announcement_note,
};
use super::runtime_session::{
    account_committed_turn_against_goal, begin_turn_resume, holistic_last_turn_usage,
    reconcile_synthesized_visibility, OpenHumanSessionState,
};
use super::{OpenHumanSessionFactory, OpenHumanSessionHooks, OpenHumanTranscriptCodec};
use crate::agent::tinyagents::host::OpenHumanRunContext;

struct RecordingDriver(Arc<Mutex<Vec<OpenHumanRunContext>>>);

#[test]
fn transcript_suppression_is_applied_before_resume() {
    let mut state = OpenHumanSessionState::default();
    state.pending_turn_overrides = super::TurnOverrides {
        suppress_transcript_autoload: true,
        suppress_tools: true,
        ..Default::default()
    };
    let mut resume = ResumeMode::LatestForAgent;

    begin_turn_resume(&mut state, &mut resume);

    assert_eq!(resume, ResumeMode::Never);
    assert!(state.pending_turn_overrides == super::TurnOverrides::default());
    assert!(state.active_turn_overrides.suppress_transcript_autoload);
    assert!(state.active_turn_overrides.suppress_tools);
}

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
    assert!(rows[0]
        .tool_failure
        .as_ref()
        .is_some_and(|failure| failure.failed));
}

/// The durable record is the parent's OWN spend.
///
/// This assertion used to read `18` — "direct + completed child input" — which
/// made the root transcript's record overlap the child's own transcript, and
/// `threads::ops::usage` added both (#6460). Inverted deliberately, not deleted:
/// the sub-agent entry is still appended below, and the point is that it does
/// **not** move these numbers. The live `chat_done` projection stays inclusive —
/// see `last_turn_usage_reports_the_same_holistic_totals_as_transcript_billing`
/// immediately below, which is the counterpart this split is meant to preserve.
#[test]
fn codec_attaches_only_this_agents_own_sidecar_usage_to_atomic_append() {
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
        usage: crate::agent::subagent_host::SubagentUsage {
            input_tokens: 5,
            output_tokens: 2,
            cached_input_tokens: 1,
            charged_amount_usd: 0.001,
        },
    });
    // `OpenHumanRunContext` is `Clone` and its child ledger is an `Arc`, so this
    // probe observes the same ledger after `context` is moved into the options.
    let probe = context.clone();
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

    // The child contributed 5/2/1/$0.001 and is deliberately absent here: it is
    // recorded in its own transcript, which the thread aggregate reads directly.
    assert_eq!(
        usage.usage.input, 13,
        "the parent's own input, NOT 18 — the child's 5 belongs to the child's record"
    );
    assert_eq!(
        usage.usage.output, 8,
        "the parent's own output, NOT 10 — the child's 2 belongs to the child's record"
    );
    assert_eq!(
        usage.usage.cached_input, 3,
        "the parent's own cache reads, NOT 4"
    );
    assert_eq!(usage.usage.context_window, 128_000);
    assert!(
        (usage.usage.cost_usd - 0.004).abs() < f64::EPSILON,
        "the parent's own cost, NOT 0.005"
    );
    // The entry is still on the sidecar — exclusivity is about what is written,
    // not about discarding the ledger the live UI projection needs.
    assert_eq!(
        probe.subagent_usage_entries().len(),
        1,
        "the child entry survives on the ledger for the live projection"
    );
    assert_eq!(usage.iteration, 2);
    // The turn's tool outcomes are NOT copied onto the usage record: it lands
    // on the final answer row, and every call is already recorded once in the
    // envelope of the assistant row that issued it.
    assert!(
        usage.tool_calls.is_empty(),
        "usage must not duplicate the turn's tool calls onto the answer row"
    );
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
            usage: crate::agent::subagent_host::SubagentUsage {
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
            usage: crate::agent::subagent_host::SubagentUsage {
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
        session_id: None,
        parent_session_id: None,
        agent_name: "adapter-test".into(),
        agent_id: Some("adapter-test".into()),
        agent_type: Some("root".into()),
        dispatcher: "test".into(),
        provider: None,
        model: None,
        created: chrono::Utc::now().to_rfc3339(),
        updated: chrono::Utc::now().to_rfc3339(),
        turn_count: 0,
        prefix_message_count: None,
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
                session: None,
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

/// Mid-conversation availability notes are prepended to the user's next
/// message. They must read as status, never as a task: the old "act on them
/// immediately" wording had the orchestrator delegate to the integrations
/// agent in the middle of an unrelated exchange and answer "what are we doing
/// with the inbox?" to "so lets do 20-30 days then?".
#[test]
fn availability_notes_are_status_not_instructions() {
    let notes = [
        integration_announcement_note(&["gmail".to_string()]).unwrap(),
        mcp_announcement_note(&["filesystem".to_string(), "github".to_string()]).unwrap(),
        skill_announcement_note(&["deploy".to_string()]).unwrap(),
    ];
    for note in &notes {
        assert!(
            !note.contains("immediately"),
            "note must not order an action: {note}"
        );
        assert!(
            note.contains("not a request") && note.contains("only use these if"),
            "note must defer to the user's message: {note}"
        );
        assert!(note.contains("Do not tell the user to reconnect or restart"));
    }
    assert!(notes[0].contains("tool_search") && notes[0].contains("gmail"));
    assert!(!notes[0].contains("delegate_to_integrations_agent"));
    assert!(notes[1].contains("tool_search") && notes[1].contains("filesystem, github"));
    assert!(!notes[1].contains("use_mcp_server"));
    assert!(notes[2].contains("run_workflow") && notes[2].contains("deploy"));
    assert!(integration_announcement_note(&[]).is_none());
    assert!(mcp_announcement_note(&[]).is_none());
    assert!(skill_announcement_note(&[]).is_none());
}

/// A sub-agent inherits its parent's thread id for correlation, but each spawn
/// is genuinely its own transcript. It must therefore not claim the
/// conversation's durable session identity, or two concurrent workers on one
/// thread would write into the same file.
#[test]
fn a_subagent_thread_binding_claims_no_session_identity() {
    let tmp = tempfile::tempdir().unwrap();
    let config = crate::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..crate::config::Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();

    let mut root =
        super::OpenHumanSessionHost::from_config_for_agent(&config, "orchestrator").unwrap();
    root.set_thread_id(Some("thread-1"));
    // The exact stem encoding (sanitisation, per-component digest, generation
    // suffix) belongs to tinyagents and is pinned by its own tests. What
    // OpenHuman owns, and what this asserts, is that a root chat session is
    // addressed by its conversation at all.
    let root_session = root
        .session_id()
        .expect("a root chat session has an identity");
    assert!(
        root_session.starts_with("thread-1"),
        "a root chat session is addressed by its conversation, got {root_session}"
    );

    let mut child =
        super::OpenHumanSessionHost::from_config_for_agent(&config, "orchestrator").unwrap();
    child.session_parent_prefix = Some("1713000000_orchestrator".into());
    child.set_thread_id(Some("thread-1"));
    assert_eq!(child.session_id(), None);
}

/// The resume hook and builder must name the same transcript locator instance.
/// A distinct locator for the same directory lets a new thread commit, then
/// rejects a cold resumed thread before inference with InvalidSessionState (#6608).
#[test]
fn a_cold_resumed_thread_can_send_again() {
    std::thread::Builder::new()
        .stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime")
                .block_on(cold_resumed_thread_can_send_again());
        })
        .expect("test thread")
        .join()
        .expect("test thread panicked");
}

async fn cold_resumed_thread_can_send_again() {
    let root = tempfile::tempdir().expect("tempdir");
    let model: Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::new(vec![
            tinyinference_llm::model::ModelResponse::assistant("first reply"),
            tinyinference_llm::model::ModelResponse::assistant("second reply"),
        ]));
    let new_host = || {
        crate::agent::SessionHostBuilder::new()
            .chat_model(model.clone())
            .tools(Vec::new())
            .workspace_dir(root.path().join("workspace"))
            .action_dir(root.path().to_path_buf())
            .tool_dispatcher(Box::new(tinytools_agent::dialect::XmlDialect))
            .build()
            .expect("session build")
    };
    let mut host = new_host();
    host.set_thread_id(Some("thread-resume-regression"));

    assert_eq!(host.turn("first message").await.unwrap(), "first reply");
    drop(host);
    let mut host = new_host();
    host.set_thread_id(Some("thread-resume-regression"));
    assert_eq!(host.turn("second message").await.unwrap(), "second reply");
}

/// The PRODUCTION turn path must wire an artifact store, rooted outside the
/// project the agent works in (#6408).
///
/// Two assertions, and both were dead code before this PR. The existing artifact
/// tests construct `TurnContextMiddleware` with `artifact_store: Some(..)`
/// themselves, so they prove the offload code works while saying nothing about
/// whether production ever reaches it — and it did not: the production
/// constructor hard-coded `None`, which is how a whole feature shipped dead.
///
/// The root matters as much as the wiring, and it is the workspace's
/// `artifacts/tool-results`, never a directory the agent works in. Rooted at
/// `action_dir` (or a descriptor's root) every oversized output was a stray file
/// in the user's project, swept into its diff. The pointer is absolute, so it
/// no longer has to match whichever root `file_read` resolves relative paths
/// against (#6483). This drives a real host with a descriptor whose root is not
/// `action_dir`, and pins that the store followed neither.
#[tokio::test]
async fn production_turn_path_wires_an_artifact_store_outside_the_project() {
    let workspace = tempfile::tempdir().expect("workspace");
    let action_dir = tempfile::tempdir().expect("tempdir");
    let turn_root = tempfile::tempdir().expect("turn root");
    assert_ne!(
        action_dir.path(),
        turn_root.path(),
        "the two roots must differ or this test cannot tell them apart"
    );

    let model: Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::new(Vec::new()));
    let mut host = crate::agent::SessionHostBuilder::new()
        .chat_model(model)
        .tools(Vec::new())
        .workspace_dir(workspace.path().to_path_buf())
        .action_dir(action_dir.path().to_path_buf())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::XmlDialect))
        .workspace_descriptor(Some(
            tinytools::WorkspaceDescriptor::new(turn_root.path().to_path_buf())
                .with_policy_id("turn-workspace"),
        ))
        .build()
        .expect("session build");

    host.ensure_runtime_session()
        .expect("ensure runtime session");

    let state = host
        .runtime_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let middleware = state
        .context_middleware
        .as_ref()
        .expect("the turn path must retain its context middleware");
    let store = middleware.artifact_store.as_ref().expect(
        "the production turn path must wire an artifact store, or oversized \
         tool results are truncated with their tail discarded (#6408)",
    );
    assert!(store.is_detached(), "the pointer must be absolute");
    assert_eq!(
        store.root(),
        crate::security::policy::tool_result_artifacts_dir(workspace.path()),
        "the store must live under the workspace, not in the project the agent edits"
    );
    assert!(!store.root().starts_with(action_dir.path()));
    assert!(!store.root().starts_with(turn_root.path()));
}

/// Without a descriptor the answer is the same: the workspace, not
/// `action_dir`. The mirror of the test above, so neither branch can regress
/// back into the project.
#[tokio::test]
async fn artifact_store_stays_in_the_workspace_without_a_descriptor() {
    let workspace = tempfile::tempdir().expect("workspace");
    let action_dir = tempfile::tempdir().expect("tempdir");
    let model: Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::new(Vec::new()));
    let mut host = crate::agent::SessionHostBuilder::new()
        .chat_model(model)
        .tools(Vec::new())
        .workspace_dir(workspace.path().to_path_buf())
        .action_dir(action_dir.path().to_path_buf())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::XmlDialect))
        .build()
        .expect("session build");

    host.ensure_runtime_session()
        .expect("ensure runtime session");

    let state = host
        .runtime_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let store = state
        .context_middleware
        .as_ref()
        .expect("context middleware")
        .artifact_store
        .as_ref()
        .expect("artifact store");
    assert_eq!(
        store.root(),
        crate::security::policy::tool_result_artifacts_dir(workspace.path())
    );
}
