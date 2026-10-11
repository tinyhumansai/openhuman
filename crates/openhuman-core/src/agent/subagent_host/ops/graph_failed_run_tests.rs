use super::*;
use tinytools::Tool;

/// Answers two tool rounds, then rejects the third request with a 400 (which the
/// harness does not retry).
struct FailsOnThirdCallProvider {
    calls: AtomicUsize,
}
#[async_trait]
impl ChatModel<()> for FailsOnThirdCallProvider {
    fn profile(&self) -> Option<&ModelProfile> {
        Some(native_tool_profile())
    }

    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n < 2 {
            Ok(tool_response(
                &format!("call-{n}"),
                "echo",
                serde_json::json!({ "msg": format!("round-{n}") }),
            ))
        } else {
            Err(tinyinference_llm::Error::Model(
                "400 Bad Request: provider boom".to_string(),
            ))
        }
    }
}

/// #6281: a failed sub-agent run persists only the round a provider accepted as
/// structured history; the round only the rejected request carried goes into the
/// failure marker as text, so a resumed sub-agent cannot replay it.
#[tokio::test]
async fn failed_subagent_run_keeps_its_unanswered_round_out_of_history() {
    let provider = Arc::new(FailsOnThirdCallProvider {
        calls: AtomicUsize::new(0),
    });
    let parent_tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![Box::new(EchoTool)]);
    let mut allowed = HashSet::new();
    allowed.insert("echo".to_string());
    let mut history = vec![TranscriptMessage::user("please echo twice")];
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let stem = "root-session__failed_run";

    let result = run_subagent_via_graph(
        crate::agent::tinyagents::TurnModelSource::from_model(provider),
        "mock-model",
        0.0,
        &mut history,
        parent_tools,
        vec![],
        vec![],
        allowed,
        10,
        None,
        None,
        "researcher",
        "task-failed",
        false,
        None,
        crate::agent::tinyagents::host::OpenHumanRunContext::new(),
        None,
        workspace.path().to_path_buf(),
        None,
        1024,
        false,
        stem,
        "mock-channel",
        AgentTokenjuiceCompression::Off,
        None,
    )
    .await;
    assert!(result.is_err(), "the rejected third call fails the run");

    use tinyagents_session::transcript;
    let path =
        transcript::resolve_keyed_transcript_path(workspace.path(), stem).expect("transcript path");
    let persisted = transcript::read_transcript(&path).expect("failed run transcript");
    let tool_rows: Vec<&str> = persisted
        .messages
        .iter()
        .filter(|message| message.role == "tool")
        .map(|message| message.content.as_str())
        .collect();
    assert_eq!(
        tool_rows.len(),
        1,
        "only the accepted round is persisted as a tool row: {tool_rows:?}"
    );
    assert!(
        tool_rows[0].contains("echoed:round-0"),
        "got: {tool_rows:?}"
    );
    let marker = &persisted.messages.last().expect("failure marker").content;
    assert!(
        marker.starts_with("[subagent run failed before completion:")
            && marker.contains("provider boom")
            && marker.contains("round-1"),
        "the marker carries the cause and the unanswered round as text, got: {marker}"
    );

    // Two model calls were answered before the rejection; the recorded
    // iteration must say so rather than count persisted messages.
    let raw = std::fs::read_to_string(&path).expect("raw transcript");
    let marker_line = raw
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .rfind(|line| line.get("role").and_then(|role| role.as_str()) == Some("assistant"))
        .expect("failure marker line");
    assert_eq!(
        marker_line.get("iteration").and_then(|n| n.as_u64()),
        Some(2),
        "the failed run reports the model calls a provider answered: {marker_line}"
    );
}

/// #6281 review: a failed run recovers the caller's original history, not the
/// provider-bound seed the snapshot started from (which carries rehydrated image
/// data for a vision run).
#[test]
fn failed_run_history_keeps_the_original_seed_not_the_provider_bound_one() {
    use crate::agent::tinyagents::TranscriptSnapshot;
    use tinyinference_llm::message::Message;

    let original = vec![TranscriptMessage::user("describe [IMAGE:attachment-1]")];
    let snapshot = TranscriptSnapshot {
        messages: vec![
            // The provider-bound seed: the placeholder expanded to image data.
            Message::user("describe data:image/png;base64,AAAA"),
            // Round 0, answered by the next request.
            Message::Assistant(
                tool_response("call-0", "echo", serde_json::json!({ "msg": "round-0" })).message,
            ),
            Message::tool("call-0", "echoed:round-0"),
            // Round 1, carried only by the failing request.
            Message::Assistant(
                tool_response("call-1", "echo", serde_json::json!({ "msg": "round-1" })).message,
            ),
            Message::tool("call-1", "echoed:round-1"),
        ],
        accepted_len: 3,
        request_base_len: 1,
        ..TranscriptSnapshot::default()
    };

    let (recovered, unanswered) =
        super::super::transcript::failed_run_history(&original, &snapshot);

    assert_eq!(
        recovered.first().map(|message| message.content.as_str()),
        Some("describe [IMAGE:attachment-1]"),
        "the durable placeholder must be kept: {recovered:?}"
    );
    assert!(
        !recovered
            .iter()
            .any(|message| message.content.contains("base64")),
        "provider-bound image data must not be persisted: {recovered:?}"
    );
    assert_eq!(
        recovered
            .iter()
            .filter(|message| message.role == "tool")
            .count(),
        1,
        "only the accepted round follows the seed: {recovered:?}"
    );
    let declaration = &recovered[1];
    assert_eq!(
        declaration.tool_calls.len(),
        1,
        "accepted native rounds retain their declaration"
    );
    assert_eq!(
        declaration.tool_calls[0].id,
        recovered[2].tool_call_id.as_deref().unwrap_or_default()
    );
    assert!(
        unanswered.is_some_and(|steps| steps.contains("round-1")),
        "the unanswered round is kept as text"
    );
}
