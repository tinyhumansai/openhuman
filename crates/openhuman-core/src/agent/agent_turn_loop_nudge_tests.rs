use super::*;
use tinyinference_llm::message::Message;

// #6725: the repeated-failure breaker's corrective nudge reaches the next model
// request only. It used to be a steering `InjectMessage`, which joined the
// working transcript and was committed as durable history.

const NUDGE: &str =
    "The last call failed validation. Correct its schema or arguments once before trying again.";
const FAILED_TOOL_RESULT_WITH_NUDGE: &str =
    "{\"status_code\": 422}\n\n[harness note]\nThe last call failed validation. Correct its schema or arguments once before trying again.";

/// A native-tool-calling model that replays scripted responses and records the
/// messages of every request it receives.
struct RecordingProvider {
    responses: Mutex<Vec<ChatResponse>>,
    requests: Mutex<Vec<Vec<Message>>>,
    /// Fail every call from this 0-based index on (drives the error path).
    fail_from_call: Option<usize>,
}

#[async_trait]
impl ChatModel<()> for RecordingProvider {
    fn profile(&self) -> Option<&ModelProfile> {
        static PROFILE: std::sync::LazyLock<ModelProfile> =
            std::sync::LazyLock::new(|| ModelProfile {
                provider: Some("agent-test".to_string()),
                tool_calling: true,
                parallel_tool_calls: true,
                ..ModelProfile::default()
            });
        Some(&PROFILE)
    }

    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        let index = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request.messages.clone());
            requests.len() - 1
        };
        if self.fail_from_call.is_some_and(|from| index >= from) {
            return Err(tinyinference_llm::Error::Model(
                "400 Bad Request: provider error".into(),
            ));
        }
        let response = {
            let mut guard = self.responses.lock().unwrap();
            if guard.is_empty() {
                text_response("done")
            } else {
                guard.remove(0)
            }
        };
        Ok(crate::agent::tinyagents::model::native_model_response_for_request(&response, &request))
    }
}

/// Fails with a tool-owned 422, which the breaker classifies as `validation`.
struct ValidationFailingTool(&'static str);

#[async_trait]
impl Tool for ValidationFailingTool {
    fn name(&self) -> &str {
        self.0
    }

    fn description(&self) -> &str {
        "Always fails validation"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        Ok(ToolResult::error(r#"{"status_code": 422}"#))
    }
}

fn call(id: &str, name: &str) -> ChatResponse {
    tool_response(vec![NativeToolCall {
        id: id.into(),
        name: name.into(),
        arguments: "{}".into(),
        extra_content: None,
    }])
}

fn nudge_count(messages: &[Message]) -> usize {
    messages
        .iter()
        .filter(|m| matches!(m, Message::Tool(_)) && m.text() == FAILED_TOOL_RESULT_WITH_NUDGE)
        .count()
}

async fn run_turn(
    script: Vec<ChatResponse>,
    tools: Vec<Box<dyn Tool>>,
) -> (Vec<Vec<Message>>, Vec<TranscriptEntry>) {
    let (requests, history, _tmp) = run_turn_failing_from(script, tools, None).await;
    (requests, history)
}

async fn run_turn_failing_from(
    script: Vec<ChatResponse>,
    tools: Vec<Box<dyn Tool>>,
    fail_from_call: Option<usize>,
) -> (Vec<Vec<Message>>, Vec<TranscriptEntry>, tempfile::TempDir) {
    let provider = Arc::new(RecordingProvider {
        responses: Mutex::new(script),
        requests: Mutex::new(Vec::new()),
        fail_from_call,
    });
    let (mut agent, tmp) = build_agent_with(provider.clone(), tools, Box::new(NativeDialect));
    let outcome = agent.turn("do the thing").await;
    assert_eq!(
        outcome.is_err(),
        fail_from_call.is_some(),
        "turn outcome: {outcome:?}"
    );
    let requests = provider.requests.lock().unwrap().clone();
    (requests, agent.history(), tmp)
}

/// No system row may follow the first conversational row of committed history.
fn committed_system_rows_past_prefix(history: &[TranscriptEntry]) -> Vec<String> {
    history
        .iter()
        .filter_map(|m| match m {
            TranscriptEntry::Chat(chat) => Some(chat),
            _ => None,
        })
        .skip_while(|chat| chat.role.as_str() == "system")
        .filter(|chat| chat.role.as_str() == "system")
        .map(|chat| chat.content.clone())
        .collect()
}

#[tokio::test]
async fn validation_nudge_reaches_the_retry_but_is_not_committed() {
    let (requests, history) = run_turn(
        vec![call("tc1", "fail_a"), text_response("recovered")],
        vec![Box::new(ValidationFailingTool("fail_a"))],
    )
    .await;

    assert_eq!(requests.len(), 2, "one failing call, then the retry");
    assert!(
        matches!(requests[1].last(), Some(message) if matches!(message, Message::Tool(_)) && message.text() == FAILED_TOOL_RESULT_WITH_NUDGE),
        "the retry request must end with the nudge, got {:?}",
        requests[1].last()
    );
    assert_eq!(nudge_count(&requests[1]), 1);
    // Fixture guard: the failed call really is in the committed turn.
    assert!(
        history.iter().any(|m| matches!(
            m,
            TranscriptEntry::Chat(chat) if chat.role.as_str() == "tool" && chat.content.contains("422")
        )) || history
            .iter()
            .any(|m| matches!(m, TranscriptEntry::ToolResults(r) if r.iter().any(|r| r.content.contains("422")))),
        "the validation failure must be part of the committed turn"
    );
    assert_eq!(
        committed_system_rows_past_prefix(&history),
        Vec::<String>::new(),
        "the nudge must not be committed into durable history"
    );
    assert_eq!(
        committed_rows_mentioning_nudge(&history),
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn a_second_validation_failure_rearms_the_nudge() {
    // Validation has a budget of one per (tool, scope): the same tool failing
    // again halts the run. A different tool's first failure is a fresh blocker
    // and must nudge again, even though the first nudge was already consumed.
    let (requests, history) = run_turn(
        vec![
            call("tc1", "fail_a"),
            call("tc2", "fail_b"),
            text_response("recovered"),
        ],
        vec![
            Box::new(ValidationFailingTool("fail_a")),
            Box::new(ValidationFailingTool("fail_b")),
        ],
    )
    .await;

    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests.iter().map(|r| nudge_count(r)).collect::<Vec<_>>(),
        [0, 1, 1],
        "each failure nudges exactly the request that follows it; a consumed nudge is not carried over"
    );
    assert!(
        matches!(requests[2].last(), Some(message) if matches!(message, Message::Tool(_)) && message.text() == FAILED_TOOL_RESULT_WITH_NUDGE),
        "the re-armed nudge must end the request after the second failure"
    );
    assert_eq!(
        committed_system_rows_past_prefix(&history),
        Vec::<String>::new()
    );
}

/// Committed rows carrying the nudge text in any role. A failed turn renders
/// its unanswered request into the failure note as text (#6281), so a leak can
/// arrive inside an assistant row, not only as a system row.
fn committed_rows_mentioning_nudge(history: &[TranscriptEntry]) -> Vec<String> {
    history
        .iter()
        .filter_map(|m| match m {
            TranscriptEntry::Chat(chat) if chat.content.contains(NUDGE) => {
                Some(format!("{}: {}", chat.role.as_str(), chat.content))
            }
            _ => None,
        })
        .collect()
}

/// Every persisted file under the workspace, as (lossy) text: the session
/// store's database files included, so a row written there is searched too.
fn persisted_text(root: &std::path::Path) -> Vec<(std::path::PathBuf, String)> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            let bytes = std::fs::read(entry.path()).ok()?;
            let text = String::from_utf8_lossy(&bytes).into_owned();
            Some((entry.into_path(), text))
        })
        .collect()
}

#[tokio::test]
async fn a_failed_nudged_request_does_not_persist_the_nudge() {
    // When the nudged request itself fails, the driver persists a display-only
    // failure record that renders the request's unanswered tail as text
    // (#6281). The injector runs after the snapshot middleware, so that tail
    // never holds the nudge.
    let (requests, _history, tmp) = run_turn_failing_from(
        vec![call("tc1", "fail_a")],
        vec![Box::new(ValidationFailingTool("fail_a"))],
        Some(1),
    )
    .await;

    assert_eq!(
        requests.len(),
        2,
        "fail_a, then the nudged request that fails"
    );
    assert_eq!(
        nudge_count(&requests[1]),
        1,
        "the nudged request reached the model"
    );
    let files = persisted_text(tmp.path());
    // Fixture guard: the failure record rendered the failing request's
    // unanswered tail, which is exactly where a leaked nudge would be written.
    assert!(
        files
            .iter()
            .any(|(_, text)| text.contains("The request that failed also carried these steps")),
        "the failure record must render the unanswered steps; files: {:?}",
        files.iter().map(|(path, _)| path).collect::<Vec<_>>()
    );
    let leaked: Vec<_> = files
        .iter()
        .filter(|(_, text)| text.contains(NUDGE))
        .map(|(path, _)| path)
        .collect();
    assert!(leaked.is_empty(), "the nudge was persisted in {leaked:?}");
}
