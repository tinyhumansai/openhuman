use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tinyagents_harness::tool::ToolSnapshot;
use tinyagents_runtime::{
    DriverFailure, DriverOutcome, DriverRequest, PrefixSnapshot, ResumeMode, SessionBuilder,
    SessionDriver, SessionTerminal, SessionTurnRequest, TranscriptCodec, TranscriptTurnOptions,
    TurnOptions,
};
use tinyagents_session::port::{InMemorySessionStores, SessionStoreProvider};
use tinyagents_session::transcript::TranscriptMeta;
use tinyinference_llm::message::Message;

use super::runtime_session::{account_committed_turn_against_goal, holistic_last_turn_usage};
use super::{OpenHumanSessionHooks, OpenHumanTranscriptCodec};
use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::integrations::announcement_notes::{
    integration_announcement_note, mcp_announcement_note, skill_announcement_note,
};

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
            outcome: None,
            tool_outcomes: Vec::new(),
        })
    }
}

#[test]
fn codec_marks_failed_tool_rows_from_the_explicit_turn_sidecar() {
    let context = OpenHumanRunContext::new();
    context.session_sidecar.lock().unwrap().tool_outcomes.push(
        tinyagents_harness::turn_outcome::ToolCallOutcome {
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
        sidecar.cost = tinyagents_harness::cost::CostTally {
            known_usd: 0.004,
            source: tinyagents_harness::cost::CostSource::Charged,
        };
        sidecar.context_window = 128_000;
        sidecar.last_call_input_tokens = 9;
        sidecar.last_call_output_tokens = 2;
        sidecar
            .tool_outcomes
            .push(tinyagents_harness::turn_outcome::ToolCallOutcome {
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
        usage: tinyagents_harness::progress::SubagentUsage {
            input_tokens: 5,
            output_tokens: 2,
            cached_input_tokens: 1,
            charged_amount_usd: 0.001,
            cost_source: tinyagents_harness::cost::CostSource::Charged,
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
    // The final call's size is persisted beside the summed spend, so a thread
    // reopened later can still fill the context gauge correctly.
    assert_eq!(
        (usage.usage.last_call_input, usage.usage.last_call_output),
        (9, 2)
    );
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
        cost: tinyagents_harness::cost::CostTally {
            known_usd: 0.004,
            source: tinyagents_harness::cost::CostSource::Charged,
        },
        context_window: 128_000,
        last_call_input_tokens: 9,
        last_call_output_tokens: 2,
        ..Default::default()
    };
    let child = crate::agent::tinyagents::host::SubagentUsageEntry {
        task_id: "child-ui".into(),
        agent_id: "researcher".into(),
        usage: crate::progress::SubagentUsage {
            input_tokens: 5,
            output_tokens: 2,
            cached_input_tokens: 1,
            charged_amount_usd: 0.001,
            cost_source: tinyagents_harness::cost::CostSource::Charged,
        },
    };
    sidecar.subagents.push(child.clone());
    let usage = holistic_last_turn_usage(&sidecar);
    assert_eq!(usage.input_tokens, 18);
    assert_eq!(usage.output_tokens, 10);
    assert_eq!(usage.cached_input_tokens, 4);
    assert!((usage.cost_usd.expect("all calls charged") - 0.005).abs() < f64::EPSILON);
    assert_eq!(usage.context_window, 128_000);
    // The gauge numerator is the root's final call alone: neither the turn's
    // summed spend nor the child's tokens, which ran in their own window.
    assert_eq!(usage.context_tokens, 11);
    assert_eq!(usage.subagents.len(), 1, "detail breakdown is retained");
}

#[test]
fn explicit_hidden_delegate_stays_hidden_across_dynamic_refreshes() {
    let visible = ["read_file".to_string()]
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

    let mut managed = tinyagents_runtime::ManagedToolNames {
        visible,
        ..Default::default()
    };
    managed.refresh_synthesized(&prior, &refreshed, false);

    assert!(
        !managed.visible.contains("delegate_research"),
        "an explicitly hidden existing delegate remains hidden after refresh"
    );
    assert!(
        !managed.visible.contains("delegate_drive"),
        "a new connection cannot bypass an explicit visibility restriction"
    );
    assert!(
        !managed.visible.contains("delegate_calendar"),
        "a revoked connection leaves the surface"
    );
}

#[test]
fn wildcard_delegate_visibility_admits_new_connections_and_removes_revoked_ones() {
    let visible = ["read_file".to_string(), "delegate_calendar".to_string()]
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let prior = ["delegate_calendar".to_string()]
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let refreshed = ["delegate_drive".to_string()]
        .into_iter()
        .collect::<std::collections::HashSet<_>>();

    let mut managed = tinyagents_runtime::ManagedToolNames {
        visible,
        ..Default::default()
    };
    managed.refresh_synthesized(&prior, &refreshed, true);

    assert!(managed.visible.contains("delegate_drive"));
    assert!(!managed.visible.contains("delegate_calendar"));
    assert!(managed.visible.contains("read_file"));
}

#[tokio::test]
async fn committed_goal_accounting_uses_direct_and_completed_child_usage() {
    let workspace = tempfile::tempdir().unwrap();
    let core = crate::core::runtime::CoreContext::for_test(
        crate::core::runtime::DomainSet::full(),
        Some(workspace.path().to_path_buf()),
    );
    crate::core::runtime::CoreContext::scope(core, async {
        let goal_store = crate::storage::sessions::stores_or_ephemeral(workspace.path()).kv;
        tinyagents_harness_tools::goals::store::set(
            &goal_store,
            "receipt-goal",
            "finish migration",
            Some(20),
        )
        .await
        .unwrap();
        assert!(
            tinyagents_harness_tools::goals::store::get(&goal_store, "receipt-goal")
                .await
                .unwrap()
                .is_some(),
            "the scoped goal store must contain the fixture before accounting"
        );
        let mut sidecar = crate::agent::tinyagents::host::run_context::SessionTurnSidecar {
            input_tokens: 9,
            output_tokens: 4,
            duration: Some(std::time::Duration::from_secs(2)),
            ..Default::default()
        };
        let child = crate::agent::tinyagents::host::SubagentUsageEntry {
            task_id: "child-goal".into(),
            agent_id: "researcher".into(),
            usage: tinyagents_harness::progress::SubagentUsage {
                input_tokens: 5,
                output_tokens: 3,
                ..Default::default()
            },
        };
        sidecar.subagents.push(child);
        account_committed_turn_against_goal(
            workspace.path(),
            &crate::core::runtime::DispatchCtx::ambient(),
            Some("receipt-goal"),
            &sidecar,
        )
        .await;

        let goal = tinyagents_harness_tools::goals::store::get(&goal_store, "receipt-goal")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            goal.tokens_used, 21,
            "receipt accounts direct + child usage"
        );
        assert_eq!(
            goal.status,
            tinyagents_harness_tools::goals::ThreadGoalStatus::BudgetLimited,
            "receipt-backed accounting advances continuation/budget state"
        );
    })
    .await;
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
        prefix_reset: false,
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
    let locator = InMemorySessionStores::new()
        .for_agent("adapter-test")
        .transcripts;
    let context = OpenHumanRunContext::new();
    let cancellation = context.cancellation.clone();
    let mut session = SessionBuilder::new(Arc::new(RecordingDriver(Arc::clone(&seen_context))))
        .codec(Arc::new(OpenHumanTranscriptCodec))
        .hooks(hooks)
        .prefix(PrefixSnapshot::default())
        .tool_snapshot(ToolSnapshot::default())
        .transcript(Arc::clone(&locator), "adapter-test", meta())
        .build()
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
                controls: Default::default(),
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
        locator.latest_for_agent("adapter-test").is_some()
            || locator.root_for_thread("thread-1").is_some(),
        "commit precedes finalization"
    );
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
#[tokio::test]
async fn a_subagent_thread_binding_claims_no_session_identity() {
    let tmp = tempfile::tempdir().unwrap();
    let config = crate::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..crate::config::Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    crate::agent::session_host::test_support::install_managed_model_fixture(&config).await;

    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let mut root =
        super::OpenHumanSessionHost::from_config_for_agent(&ctx, &config, "orchestrator")
            .await
            .unwrap();
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
        super::OpenHumanSessionHost::from_config_for_agent(&ctx, &config, "orchestrator")
            .await
            .unwrap();
    child.session_parent_prefix = Some("1713000000_orchestrator".into());
    child.set_thread_id(Some("thread-1"));
    assert_eq!(child.session_id(), None);
}

/// The resume hook and builder must name the same transcript locator instance.
/// A distinct locator for the same directory lets a new thread commit, then
/// rejects a cold resumed thread before inference with InvalidSessionState (#6608).
#[path = "runtime_adapter_continuation_tests.rs"]
mod continuation_tests;
