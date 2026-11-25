// Every child is detached. Chat parents receive a later delivery; headless
// callers retain the result in their run log.
fn accepted_spawn_result(
    task_id: &str,
    session_id: &str,
    agent_id: &str,
    worker_thread_id: Option<&str>,
    reused: bool,
    reuse_decision: &str,
    task_key: &str,
) -> ToolResult {
    let payload = async_subagent_ref_payload(
        task_id,
        session_id,
        task_key,
        agent_id,
        worker_thread_id,
        reused,
        reuse_decision,
        "running",
    );
    let payload_json = match serde_json::to_string(&payload) {
        Ok(serialized) => {
            log::debug!(
                "[spawn_subagent] serialized async reference payload bytes={}",
                serialized.len()
            );
            serialized
        }
        Err(error) => {
            log::debug!(
                "[spawn_subagent] failed to serialize async reference payload: {}",
                error
            );
            "{}".to_string()
        }
    };
    log::debug!("[spawn_subagent] formatting accepted response");
    ToolResult::success(format_async_subagent_accepted(
        payload["agent_id"].as_str().unwrap_or("subagent"),
        &payload_json,
    ))
}

fn refuse_empty_spawn_args(agent_id: &str, prompt: &str) -> Option<ToolResult> {
    if agent_id.is_empty() {
        return Some(ToolResult::error(
            "spawn_subagent: `agent_id` is required",
        ));
    }
    if prompt.is_empty() {
        return Some(ToolResult::error(
            "spawn_subagent: `prompt` is required",
        ));
    }
    None
}
