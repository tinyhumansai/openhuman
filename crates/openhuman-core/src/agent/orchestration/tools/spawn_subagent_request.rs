// Argument decoding for `spawn_subagent`.
//
// Textually included into `spawn_subagent_surface.rs` beside
// `spawn_subagent_host_impl.rs`, so it shares that module's imports. Split
// out because the execute fragment had grown to 847 lines against the layout
// gate's pin, and this prologue is the one phase of that function that is a
// pure function of `args` — every other phase closes over the run context, the
// registry or the session store.
//
// It is also the only seam in that function whose types can all be named.
// `find_reusable` yields a `DurableSubagentSession` from a private `types`
// module whose re-export is `#[cfg(test)]`-gated, so the reuse phase — the
// larger and more tempting candidate — cannot be given a signature without
// widening another module's surface. That is left as follow-up rather than
// forced through here.

/// The `spawn_subagent` arguments, decoded and normalised.
///
/// Trimmed, with blank strings treated as absent, exactly as the inline code
/// did — `task_title` keeps its default and `task_key` is normalised through
/// `subagent_sessions`.
struct AsyncSpawnArgs {
    agent_id: String,
    prompt: String,
    context: Option<String>,
    model_override: Option<String>,
    task_title: String,
    task_key: String,
    force_fresh: bool,
}

fn decode_async_spawn_args(args: &serde_json::Value) -> AsyncSpawnArgs {
    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let prompt = args
        .get("prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let context = args
        .get("context")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let model_override = args
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let task_title = args
        .get("task_title")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Background subagent")
        .to_string();
    let task_key_source = durable_task_key_source(args, &prompt, context.as_deref());
    let task_key = subagent_sessions::normalize_task_key(&task_key_source);
    let force_fresh = args.get("fresh").and_then(|v| v.as_bool()).unwrap_or(false);
    AsyncSpawnArgs {
        agent_id,
        prompt,
        context,
        model_override,
        task_title,
        task_key,
        force_fresh,
    }
}
