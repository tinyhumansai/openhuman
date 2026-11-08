//! The `[context]` history-replay options a session pins reach the model's
//! request: a thread with tool result expiry on sends an old tool result as a
//! one-line note, not in full. The session prepares the options on its run
//! config; the driver has to hand them to the run loop that reads them.

use std::sync::Arc;

use serde_json::{json, Value};
use tinyagents_harness::testkit::ScriptedModel;
use tinyinference_llm::message::Message;
use tinyinference_llm::model::{ChatModel, ModelProfile, ModelResponse};

use super::transcript_compat_tests::{call, response, run_async, store_locator, thread_id};

const TOOL_BODY: &str = "the echoed lines of a long listing";

struct EchoTool;

#[async_trait::async_trait]
impl tinytools::Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "echoes"
    }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {"q": {"type": "string"}}})
    }
    async fn execute(&self, _args: Value) -> anyhow::Result<tinytools::ToolResult> {
        Ok(tinytools::ToolResult::success(TOOL_BODY))
    }
}

fn host_with_context(
    root: &std::path::Path,
    model: Arc<ScriptedModel>,
    thread: &str,
    context: crate::config::ContextSettings,
) -> crate::agent::OpenHumanSessionHost {
    let mut config = crate::config::Config::default();
    config.workspace_dir = root.join("workspace");
    config.action_dir = root.to_path_buf();
    config.config_path = root.join("config.toml");
    config.modules.enabled = false;
    config.context = context;
    // No TinySecurity module runs in a unit test, so the screen would hold the input.
    config.prompt_screen.mode = crate::config::schema::PromptScreenMode::Off;
    let chat_model: Arc<dyn ChatModel<()>> = model;
    let mut host = crate::agent::SessionHostBuilder::new()
        .with_session_history_locator(store_locator(root))
        .model_name("test/session-fixture".to_string())
        .chat_model_with_config(chat_model, Arc::new(config))
        .tools(vec![Box::new(EchoTool)])
        .tool_dispatcher(Box::new(tinytools_agent::dialect::NativeDialect))
        .build()
        .expect("session build");
    host.set_thread_id(Some(thread));
    host
}

/// The text of every tool result in the request, oldest first.
fn tool_results(request: &tinyinference_llm::model::ModelRequest) -> Vec<String> {
    request
        .messages
        .iter()
        .filter(|message| matches!(message, Message::Tool(_)))
        .map(Message::text)
        .collect()
}

/// Runs a tool-using first turn and `later` plain turns on one thread, and
/// returns the tool results each later turn's request carried.
async fn tool_results_per_later_turn(
    context: crate::config::ContextSettings,
    later: usize,
) -> Vec<Vec<String>> {
    // No TinySecurity module runs in a unit test: with it enabled the redactor
    // would replace every tool result.
    let _off = crate::security::switch::off_for_this_thread();
    let root = tempfile::tempdir().expect("tempdir");
    let thread = thread_id("expiry-wire");
    let mut script = vec![
        response(Vec::new(), vec![call("call-1", "echo", json!({"q": "x"}))]),
        ModelResponse::assistant("first done"),
    ];
    script.extend((0..later).map(|_| ModelResponse::assistant("ok")));
    let model = Arc::new(ScriptedModel::new(script).with_profile(ModelProfile {
        tool_calling: true,
        ..Default::default()
    }));
    let mut host = host_with_context(root.path(), model.clone(), &thread, context);
    host.turn("first").await.expect("first turn");
    let mut per_turn = Vec::new();
    for turn in 0..later {
        let before = model.requests().len();
        host.turn(&format!("later {turn}"))
            .await
            .expect("later turn");
        let requests = model.requests();
        per_turn.push(tool_results(&requests[before]));
    }
    per_turn
}

/// One result, carried whole: the tool's body, not a note.
fn assert_full(results: &[String]) {
    assert_eq!(results.len(), 1);
    assert!(
        results[0].contains(TOOL_BODY) && !results[0].starts_with("[used "),
        "the result must ride along in full, got {:?}",
        results[0]
    );
}

#[test]
fn an_old_tool_result_goes_out_as_a_note_once_it_is_past_max_age_turns() {
    run_async(async {
        let mut context = crate::config::ContextSettings::default();
        context.tool_result_max_age_turns = Some(2);
        let per_turn = tool_results_per_later_turn(context, 3).await;

        // Two user turns after the result is within the age: sent in full.
        assert_full(&per_turn[0]);
        assert_full(&per_turn[1]);
        // A third later turn puts it past `max_age_turns = 2`.
        assert_eq!(per_turn[2].len(), 1);
        assert!(
            per_turn[2][0].starts_with("[used echo:"),
            "the old result must be replaced by a note, got {:?}",
            per_turn[2][0]
        );
    });
}

#[test]
fn a_tool_result_is_sent_in_full_while_expiry_is_off() {
    run_async(async {
        let per_turn =
            tool_results_per_later_turn(crate::config::ContextSettings::default(), 3).await;
        for results in &per_turn {
            assert_full(results);
        }
    });
}
