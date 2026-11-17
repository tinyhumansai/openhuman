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
    host_with_context_and_contract(root, model, thread, context, None)
}

fn host_with_context_and_contract(
    root: &std::path::Path,
    model: Arc<ScriptedModel>,
    thread: &str,
    context: crate::config::ContextSettings,
    required_output: Option<crate::config::RequiredOutputContract>,
) -> crate::agent::OpenHumanSessionHost {
    let mut config = crate::config::Config::default();
    config.agent.required_output = required_output;
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
        .config(config.agent.clone())
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

/// A thread whose durable history was imported has rows but no frozen prefix
/// and no recorded options: it predates the pin, so it keeps today's replay
/// (nothing expires) whatever `[context]` says now.
#[test]
fn a_thread_with_imported_history_and_no_record_keeps_todays_replay() {
    use crate::agent::tinyagents::replay_pin::REPLAY_METADATA_KEY;
    use tinyagents_session::transcript::TranscriptMessage;

    run_async(async {
        let root = tempfile::tempdir().expect("tempdir");
        let thread = thread_id("imported-unpinned-replay");
        let model = Arc::new(
            ScriptedModel::new(vec![ModelResponse::assistant("ok")]).with_profile(ModelProfile {
                tool_calling: true,
                ..Default::default()
            }),
        );
        let mut context = crate::config::ContextSettings::default();
        context.tool_result_max_age_turns = Some(2);
        let mut host = host_with_context(root.path(), model, &thread, context);
        let imported = [
            TranscriptMessage::user("legacy question"),
            TranscriptMessage::assistant("legacy answer"),
        ];
        assert!(host.adopt_imported_history(&imported).expect("adopt"));
        host.turn("hello").await.expect("turn");
        let session = host.session.clone().expect("bound session");
        let records: usize =
            super::transcript_compat_tests::store_tree(root.path(), &thread, &session)
                .values()
                .map(|stored| stored.to_string().matches(REPLAY_METADATA_KEY).count())
                .sum();
        assert_eq!(
            records, 0,
            "the config of today must not be adopted by a thread that predates the pin"
        );
    });
}

/// A required-output repair is a tools-off request built after the loop. On a
/// thread with expiry at one user turn, a tool turn, a plain turn and a second
/// tool turn leave the first result stale when the third turn's reply, which
/// lacks the required block, is repaired.
#[test]
fn the_required_output_repair_request_carries_expired_results_as_notes() {
    run_async(async {
        let _off = crate::security::switch::off_for_this_thread();
        let root = tempfile::tempdir().expect("tempdir");
        let thread = thread_id("expiry-repair");
        let repaired = ModelResponse::assistant("{\"answer\":\"ok\"}");
        // Every reply lacks the block, so every turn is repaired once.
        let script = vec![
            response(Vec::new(), vec![call("call-1", "echo", json!({"q": "x"}))]),
            ModelResponse::assistant("first done"),
            repaired.clone(),
            ModelResponse::assistant("second done"),
            repaired.clone(),
            response(Vec::new(), vec![call("call-2", "echo", json!({"q": "y"}))]),
            ModelResponse::assistant("third done"),
            repaired,
        ];
        let model = Arc::new(ScriptedModel::new(script).with_profile(ModelProfile {
            tool_calling: true,
            ..Default::default()
        }));
        let mut context = crate::config::ContextSettings::default();
        context.tool_result_max_age_turns = Some(1);
        let mut host = host_with_context_and_contract(
            root.path(),
            model.clone(),
            &thread,
            context,
            Some(crate::config::RequiredOutputContract::new("answer")),
        );
        for turn in ["first", "second", "third"] {
            host.turn(turn).await.expect("turn");
        }
        let requests = model.requests();
        let repair = requests.last().expect("the third turn's repair request");
        assert!(repair.tools.is_empty(), "the last request is the repair");
        let results = tool_results(repair);
        assert_eq!(results.len(), 2, "{results:?}");
        assert!(
            results[0].starts_with("[used echo:"),
            "the stale result must go out as a note, got {:?}",
            results[0]
        );
        assert!(
            results[1].contains(TOOL_BODY),
            "this turn's own result stays whole, got {:?}",
            results[1]
        );
    });
}
