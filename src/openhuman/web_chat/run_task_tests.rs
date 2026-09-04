use super::*;

fn ok() -> Result<WebChatTaskResult, String> {
    Ok(WebChatTaskResult {
        full_response: "hello".to_string(),
        citations: Vec::new(),
        usage: None,
        workspace_dir: std::path::PathBuf::from("/tmp/ws"),
    })
}

#[test]
fn poisoned_on_managed_sse_bad_request_frame() {
    // Managed backend 400: flushed HTTP 200, then an in-stream SSE error
    // frame stamped errorCode:"BAD_REQUEST" — the exact shape the de-poison
    // guard must catch (no HTTP 400 status anywhere in the string). Payload
    // mirrors the real backend frame verified against tinyhumansai/backend
    // upstream/develop `routes/inference.ts::writeInferenceSSE`
    // ({error:{message,type:"stream_error",errorCode}}), wrapped by the
    // client's `sse_error_frame_bail_message` as
    // "OpenHuman streaming API error: <payload>". `validateToolMessageOrdering`
    // throws BadRequestError (errorCode=BAD_REQUEST) for an orphaned tool_call_id.
    let err: Result<WebChatTaskResult, String> = Err(
        "OpenHuman streaming API error: {\"error\":{\"message\":\"Message has tool role, \
         but there was no previous assistant message with a tool call!\",\
         \"type\":\"stream_error\",\"errorCode\":\"BAD_REQUEST\"}}"
            .to_string(),
    );
    assert!(turn_result_poisoned_session(&err));
}

#[test]
fn poisoned_on_byo_provider_tool_ordering_400() {
    // BYO/direct provider tool-ordering rejection — classifies as a
    // *retryable* provider_request_rejected (poisoned history), so it evicts.
    let err: Result<WebChatTaskResult, String> = Err(
        "OpenAI API error (400 Bad Request): {\"error\":{\"message\":\"Invalid parameter: \
         messages with role 'tool' must be a response to a preceding message with \
         'tool_calls'.\"}}"
            .to_string(),
    );
    assert!(turn_result_poisoned_session(&err));
}

#[test]
fn genuine_param_400_keeps_warm_session() {
    // A non-poisoning model/parameter 400 is a *non-retryable*
    // provider_request_rejected — narrowing on `&& retryable` must keep its
    // warm session (resending the same params won't help; no reseed needed).
    let err: Result<WebChatTaskResult, String> = Err(
        "custom_openai API error (400 Bad Request): {\"error\":{\"message\":\
         \"Unsupported value: 'temperature' must be 1 for this model\"}}"
            .to_string(),
    );
    assert!(
        !turn_result_poisoned_session(&err),
        "non-retryable param 400 is not poisoned history — keep warm session"
    );
}

#[test]
fn transient_failures_keep_warm_session() {
    for raw in [
        // rate limit / 429 — history is fine, user should retry warm
        "OpenAI API error (429 Too Many Requests): slow down",
        // timeout
        "request timed out while reading response",
        // upstream 5xx
        "OpenAI API error (503 Service Unavailable): no healthy upstream",
        // session expiry — not a payload problem
        "SESSION_EXPIRED: backend session not active — sign in to resume LLM work",
    ] {
        let err: Result<WebChatTaskResult, String> = Err(raw.to_string());
        assert!(
            !turn_result_poisoned_session(&err),
            "transient/non-payload error must keep warm session: {raw}"
        );
    }
}

#[test]
fn success_keeps_warm_session() {
    assert!(!turn_result_poisoned_session(&ok()));
}
