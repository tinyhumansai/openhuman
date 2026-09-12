
async fn threads_purge_inner(
    dir: PathBuf,
) -> Result<RpcOutcome<ApiEnvelope<PurgeConversationThreadsResponse>>, String> {
    let stats = conversations::blocking::purge_threads(dir.clone()).await?;
    // No parent thread survives a purge, so cancel every detached sub-agent and
    // wipe every queued result. Same ordering as `thread_delete`: abort the
    // in-flight runs first, then clear the delivery queue. Tombstone each
    // cancelled sub-agent's thread BEFORE the final wipe so a straggler that
    // wins the cooperative-abort race (records after the wipe) is still dropped
    // by `record_completion` rather than delivered into a purged thread.
    use crate::openhuman::agent::orchestration::{background_completions, running_subagents};
    let cancelled_threads = running_subagents::cancel_all();
    let mut discarded = 0;
    for thread_id in &cancelled_threads {
        discarded += background_completions::discard_for_thread(thread_id);
    }
    discarded += background_completions::clear_all();
    log::debug!(
        "[threads] threads_purge cancelled_threads={} discarded_completions={}",
        cancelled_threads.len(),
        discarded
    );
    // Threads are gone, so any orphan turn snapshots can never be
    // reattached to a live thread. Wipe them in the same call so
    // `turn_state_list` returns an empty set after a purge. Use the
    // parse-independent `clear_all` so corrupted / half-written
    // snapshot files (which `list()` would warn-and-skip) are also
    // removed — a destructive cleanup must not leave behind anything
    // it failed to deserialize. Failures surface as RPC errors.
    turn_state::store::clear_all(dir.clone())
        .map_err(|err| format!("threads purged but turn-snapshot cleanup failed: {err}"))?;
    Ok(envelope(
        PurgeConversationThreadsResponse {
            messages_deleted: stats.message_count,
            agent_threads_deleted: stats.thread_count,
            agent_messages_deleted: stats.message_count,
        },
        None,
        None,
    ))
}

/// Returns the persisted in-flight turn snapshot for a thread, if any.
pub async fn turn_state_get(
    request: GetTurnStateRequest,
) -> Result<RpcOutcome<ApiEnvelope<GetTurnStateResponse>>, String> {
    let dir = workspace_dir().await?;
    let turn_state = turn_state::store::get(dir, &request.thread_id)?;
    let present = turn_state.is_some();
    Ok(envelope(
        GetTurnStateResponse { turn_state },
        Some(counts([("present", usize::from(present))])),
        None,
    ))
}

/// Lists every persisted turn snapshot — used by the UI on cold boot to
/// surface interrupted turns from a previous process.
pub async fn turn_state_list(
    _request: EmptyRequest,
) -> Result<RpcOutcome<ApiEnvelope<ListTurnStatesResponse>>, String> {
    let dir = workspace_dir().await?;
    let turn_states = turn_state::store::list(dir)?;
    let count = turn_states.len();
    Ok(envelope(
        ListTurnStatesResponse { turn_states, count },
        Some(counts([("num_turn_states", count)])),
        None,
    ))
}

/// Lists every persisted turn snapshot for one thread, newest first — the
/// per-turn history that lets the UI render each answer's own process trail.
pub async fn turn_state_history(
    request: GetTurnStateRequest,
) -> Result<RpcOutcome<ApiEnvelope<ListTurnStatesResponse>>, String> {
    let dir = workspace_dir().await?;
    let turn_states = turn_state::store::list_thread(dir, &request.thread_id)?;
    let count = turn_states.len();
    Ok(envelope(
        ListTurnStatesResponse { turn_states, count },
        Some(counts([("num_turn_states", count)])),
        None,
    ))
}

/// Returns one specific turn of a thread by its producing request id — used by
/// the UI to lazily load a past turn's full timeline when its insights block is
/// first expanded.
pub async fn turn_state_get_turn(
    request: GetTurnStateForRequestRequest,
) -> Result<RpcOutcome<ApiEnvelope<GetTurnStateResponse>>, String> {
    let dir = workspace_dir().await?;
    let turn_state = turn_state::store::get_turn(dir, &request.thread_id, &request.request_id)?;
    let present = turn_state.is_some();
    Ok(envelope(
        GetTurnStateResponse { turn_state },
        Some(counts([("present", usize::from(present))])),
        None,
    ))
}

/// Clears the persisted turn snapshot for a thread (e.g. after the user
/// dismisses an "interrupted" banner).
pub async fn turn_state_clear(
    request: ClearTurnStateRequest,
) -> Result<RpcOutcome<ApiEnvelope<ClearTurnStateResponse>>, String> {
    let dir = workspace_dir().await?;
    let cleared = turn_state::store::delete(dir, &request.thread_id)?;
    Ok(envelope(ClearTurnStateResponse { cleared }, None, None))
}

/// Request for [`token_usage`]: the thread whose persisted usage to total.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ThreadTokenUsageRequest {
    pub thread_id: String,
}

/// Request for [`transcript_get`]: the thread to project, plus newest-first
/// pagination controls. `cursor` is the opaque token from a prior page's
/// `nextCursor`; `limit` defaults to one screen.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TranscriptGetRequest {
    pub thread_id: String,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Project a thread's settled transcript (derived from `session_raw/*.jsonl`)
/// into typed display items, newest-first paginated. Returns an empty page with
/// `hasTranscript: false` when the thread has no persisted transcript yet.
pub async fn transcript_get(
    request: TranscriptGetRequest,
) -> Result<
    RpcOutcome<ApiEnvelope<crate::openhuman::threads::transcript_view::TranscriptPage>>,
    String,
> {
    let dir = workspace_dir().await?;
    let thread_id = request.thread_id.trim();
    if thread_id.is_empty() {
        return Err("thread_id is required".to_string());
    }
    let page = crate::openhuman::threads::transcript_view::get_page(
        &dir,
        thread_id,
        request.cursor.as_deref(),
        request.limit,
    );
    let counts = counts([
        ("items", page.items.len()),
        ("total", page.total),
        ("has_transcript", usize::from(page.has_transcript)),
    ]);
    let pagination = Some(PaginationMeta {
        limit: request
            .limit
            .unwrap_or(crate::openhuman::threads::transcript_view::DEFAULT_LIMIT),
        offset: request
            .cursor
            .as_deref()
            .and_then(|c| c.trim().parse::<usize>().ok())
            .unwrap_or(0),
        count: page.total,
    });
    Ok(envelope(page, Some(counts), pagination))
}

/// Aggregated token/cost usage for one thread, read back from its persisted
/// session transcripts. Seeds the UI footer when the user selects a thread so
/// the totals reflect prior turns instead of starting at zero.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ThreadTokenUsageResponse {
    pub thread_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub cost_usd: f64,
    pub turn_count: usize,
    /// Tokens of the most recent turn — numerator for the context-window gauge.
    pub last_turn_input_tokens: u64,
    pub last_turn_output_tokens: u64,
    /// Context window (tokens) inferred from the last model; `0` when unknown.
    pub context_window: u64,
    pub model: Option<String>,
    pub updated: Option<String>,
    /// `false` when the thread has no persisted turns yet (all zeros).
    pub has_usage: bool,
    /// Per-archetype sub-agent spend (re-audited at current pricing). The
    /// top-level totals already include this; it's broken out for the UI's
    /// per-agent footer rows.
    pub subagents: Vec<SubagentUsageDto>,
}

/// One sub-agent archetype's contribution within a thread.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SubagentUsageDto {
    pub agent_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub runs: usize,
}

/// Total a thread's persisted token/cost usage across its root transcripts.
pub async fn token_usage(
    request: ThreadTokenUsageRequest,
) -> Result<RpcOutcome<ApiEnvelope<ThreadTokenUsageResponse>>, String> {
    let dir = workspace_dir().await?;
    let summary = crate::openhuman::agent::harness::session::transcript::read_thread_usage_summary(
        &dir,
        &request.thread_id,
    );

    // Re-audit cost at CURRENT pricing rather than trusting the
    // `charged_amount_usd` persisted in the transcript: those values were
    // stamped at turn time and don't reflect later tier-pricing corrections.
    // Recompute from the persisted token counts using the last-known model's
    // rates; falls back to `fallback` only when the model is unknown.
    let audit_cost =
        |model: Option<&str>, input: u64, output: u64, cached: u64, fallback: f64| match model {
            Some(m) => crate::openhuman::agent::cost::estimate_call_cost_usd(
                m,
                &crate::openhuman::inference::provider::UsageInfo {
                    input_tokens: input,
                    output_tokens: output,
                    cached_input_tokens: cached,
                    ..Default::default()
                },
            ),
            None => fallback,
        };

    let response = match summary {
        Some(s) => {
            let context_window = s
                .model
                .as_deref()
                .and_then(crate::openhuman::inference::model_context::context_window_for_model)
                .unwrap_or(0);

            // Orchestrator (root) spend, re-audited.
            let orchestrator_cost = audit_cost(
                s.model.as_deref(),
                s.input_tokens,
                s.output_tokens,
                s.cached_input_tokens,
                s.cost_usd,
            );

            // Sub-agent archetypes, each re-audited with its own model. Older
            // sub-agent transcripts didn't persist a model on their messages, so
            // fall back to the thread's (root) model rather than pricing them at
            // $0 — sub-agents usually run on the same managed tier as the parent.
            let mut subagents = Vec::with_capacity(s.subagents.len());
            let (mut sub_in, mut sub_out, mut sub_cached, mut sub_cost) = (0u64, 0u64, 0u64, 0.0);
            for g in &s.subagents {
                let sub_model = g.model.as_deref().or(s.model.as_deref());
                let cost = audit_cost(
                    sub_model,
                    g.input_tokens,
                    g.output_tokens,
                    g.cached_input_tokens,
                    0.0,
                );
                sub_in = sub_in.saturating_add(g.input_tokens);
                sub_out = sub_out.saturating_add(g.output_tokens);
                sub_cached = sub_cached.saturating_add(g.cached_input_tokens);
                sub_cost += cost;
                subagents.push(SubagentUsageDto {
                    agent_id: g.agent_id.clone(),
                    input_tokens: g.input_tokens,
                    output_tokens: g.output_tokens,
                    cost_usd: cost,
                    runs: g.runs,
                });
            }

            // Top-level totals = orchestrator + all sub-agents.
            ThreadTokenUsageResponse {
                thread_id: request.thread_id.clone(),
                input_tokens: s.input_tokens.saturating_add(sub_in),
                output_tokens: s.output_tokens.saturating_add(sub_out),
                cached_input_tokens: s.cached_input_tokens.saturating_add(sub_cached),
                cost_usd: orchestrator_cost + sub_cost,
                turn_count: s.turn_count,
                last_turn_input_tokens: s.last_turn_input_tokens,
                last_turn_output_tokens: s.last_turn_output_tokens,
                context_window,
                model: s.model,
                updated: Some(s.updated),
                has_usage: true,
                subagents,
            }
        }
        None => ThreadTokenUsageResponse {
            thread_id: request.thread_id.clone(),
            input_tokens: 0,
            output_tokens: 0,
            cached_input_tokens: 0,
            cost_usd: 0.0,
            turn_count: 0,
            last_turn_input_tokens: 0,
            last_turn_output_tokens: 0,
            context_window: 0,
            model: None,
            updated: None,
            has_usage: false,
            subagents: Vec::new(),
        },
    };

    let has_usage = response.has_usage;
    Ok(envelope(
        response,
        Some(counts([("has_usage", usize::from(has_usage))])),
        None,
    ))
}
