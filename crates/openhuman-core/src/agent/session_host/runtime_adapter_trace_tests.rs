use std::sync::Arc;

use crate::storage::sessions::splice_fixture_tests::{head_of, memory_locator};
use async_trait::async_trait;
use tinyinference_llm::model::{ChatModel, ModelProfile, ModelRequest, ModelResponse};

struct GatedRepairModel {
    calls: std::sync::atomic::AtomicUsize,
    repair_started: tokio::sync::Notify,
    repair_call_id: std::sync::Mutex<Option<String>>,
    user_messages: std::sync::Mutex<Vec<String>>,
}

#[async_trait]
impl ChatModel<()> for GatedRepairModel {
    fn profile(&self) -> Option<&ModelProfile> {
        static PROFILE: std::sync::LazyLock<ModelProfile> =
            std::sync::LazyLock::new(|| ModelProfile {
                provider: Some("openai".into()),
                tool_calling: true,
                ..ModelProfile::default()
            });
        Some(&PROFILE)
    }

    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        self.user_messages.lock().unwrap().extend(
            request
                .messages
                .iter()
                .filter(|message| matches!(message, tinyinference_llm::message::Message::User(_)))
                .map(tinyinference_llm::message::Message::text),
        );
        let supplemental = request
            .correlation
            .as_ref()
            .filter(|correlation| correlation.call_id.contains("-host-supplemental-"));
        if let Some(correlation) = supplemental {
            *self.repair_call_id.lock().unwrap() = Some(correlation.call_id.clone());
            self.repair_started.notify_one();
            return std::future::pending().await;
        }
        if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            return Ok(ModelResponse::assistant(
                "The answer omits its required block.",
            ));
        }
        Ok(ModelResponse::assistant(
            "The answer still omits its block.",
        ))
    }
}

/// Cancellation after harness completion must finalize the durable run as
/// cancelled once Tiny has saved the partial transcript.
#[tokio::test]
async fn cancellation_during_required_output_repair_finalizes_native_run() {
    use tinyagents_harness::events::AgentEvent;
    use tinyagents_harness::ids::ExecutionStatus;
    use tinyagents_harness::terminal::TerminalReason;
    use tinyagents_harness::HarnessStatusStore;

    let scratch = tempfile::tempdir().expect("isolated workspace");
    let workspace = scratch.path().join("workspace");
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.clone();
    config.agent.required_output = Some(crate::config::RequiredOutputContract::new("answer"));
    config.components.juice = false;
    let core = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config.clone(),
    );
    let model = Arc::new(GatedRepairModel {
        calls: std::sync::atomic::AtomicUsize::new(0),
        repair_started: tokio::sync::Notify::new(),
        repair_call_id: std::sync::Mutex::new(None),
        user_messages: std::sync::Mutex::new(Vec::new()),
    });
    let chat_model: Arc<dyn ChatModel<()>> = model.clone();
    let locator = memory_locator();
    let mut host = crate::agent::SessionHostBuilder::new()
        .chat_model_with_config(chat_model, Arc::new(config.clone()))
        .model_name("test/session-fixture".to_string())
        .config(config.agent.clone())
        .tools(Vec::new())
        .workspace_dir(workspace.clone())
        .action_dir(scratch.path().to_path_buf())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::NativeDialect))
        .with_session_history_locator(Arc::clone(&locator))
        .build()
        .expect("host session");
    let thread_id = "trace-terminal-cancel-repair";
    let request_id = uuid::Uuid::new_v4().to_string();
    host.set_thread_id(Some(thread_id));
    let cancellation = tinyagents_harness::cancel::CancellationToken::new();
    host.set_host_env(crate::agent::turn_env::HostTurnEnv {
        cancellation: Some(cancellation.clone()),
        core: Some(Arc::downgrade(&core)),
        ..Default::default()
    });

    let origin = crate::core::runtime::turn_origin::AgentTurnOrigin::WebChat {
        thread_id: thread_id.into(),
        client_id: "trace-terminal-test".into(),
        request_id: Some(request_id.clone()),
    };
    let secret: String = [
        's', 'k', '-', 'r', 'e', 'p', 'a', 'i', 'r', '-', '0', '1', '2', '3', '4', '5', '6', '7',
        '8', '9', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p',
    ]
    .into_iter()
    .collect();
    let input = format!("hello using this synthetic credential: {secret}");
    let redacted = tinyinference_core::sanitize::scrub_credentials(&input);
    let turn = tokio::spawn(async move { host.run_single_with_origin(&input, Some(origin)).await });
    model.repair_started.notified().await;
    let repair_call_id = model
        .repair_call_id
        .lock()
        .unwrap()
        .clone()
        .expect("only a post-harness supplemental call releases this barrier");
    cancellation.cancel();
    assert!(turn.await.expect("turn task").is_err());
    assert!(model
        .user_messages
        .lock()
        .unwrap()
        .iter()
        .any(|message| message.contains(&redacted)));
    assert!(model
        .user_messages
        .lock()
        .unwrap()
        .iter()
        .all(|message| !message.contains(&secret)));
    let persisted = head_of(&locator, thread_id);
    let persisted_json = serde_json::to_string(&persisted.messages).expect("serialize transcript");
    assert!(!persisted_json.contains(&secret));
    assert!(persisted_json.contains(&redacted));

    let dispatch = crate::core::runtime::DispatchCtx::new(Arc::clone(&core));
    let run_id =
        crate::agent::tinyagents::journal::take_request_journal_run(&dispatch, &request_id)
            .expect("the web-chat request maps to its actual native run");
    let observations = crate::agent::tinyagents::journal::read_run_events(&dispatch, &run_id, 0)
        .await
        .expect("durable native event stream");
    let failure = observations
        .iter()
        .position(|record| {
            matches!(
                &record.event,
                AgentEvent::RunFailed { run_id: failed, outcome: Some(outcome), .. }
                    if failed.as_str() == run_id && outcome.reason == TerminalReason::Cancelled
            )
        })
        .expect("the durable run keeps its actual cancellation outcome");
    let finalized = observations
        .iter()
        .position(|record| {
            matches!(
                &record.event,
                AgentEvent::RunFinalized { run_id: finalized } if finalized.as_str() == run_id
            )
        })
        .expect("the native run is finalized");
    let completed = observations
        .iter()
        .position(|record| {
            matches!(
                &record.event,
                AgentEvent::RunCompleted { run_id: completed, .. } if completed.as_str() == run_id
            )
        })
        .expect("the harness completed before host repair");
    let repair_started = observations
        .iter()
        .position(|record| {
            matches!(
                &record.event,
                AgentEvent::ModelStarted { call_id, .. } if call_id.as_str() == repair_call_id
            )
        })
        .expect("the correlated post-harness repair is journaled");
    assert!(
        repair_started > completed,
        "repair follows harness completion"
    );
    assert!(finalized > failure, "finalization follows cancellation");
    assert!(observations.iter().any(|record| matches!(
        &record.event,
        AgentEvent::RunCompleted { run_id: completed, .. } if completed.as_str() == run_id
    )));
    let status = crate::storage::sessions::status_store_for(&workspace)
        .expect("status store")
        .get_status(&run_id)
        .await
        .expect("status lookup")
        .expect("run status persisted");
    assert_eq!(status.status, ExecutionStatus::Cancelled);
}
