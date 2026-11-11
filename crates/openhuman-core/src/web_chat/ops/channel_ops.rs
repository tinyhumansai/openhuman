//! Public RPC-facing operations for the web channel: sending a chat message,
//! cancelling in-flight turns (primary and parallel, scoped or unscoped), and
//! inspecting/clearing a thread's run queue. The turns and their queues live
//! in the context's turn runner; these are the thin entry points over it.

use serde_json::{json, Value};

use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;
use crate::web_chat::event_context;
use crate::web_chat::WebChannelEvent;

use super::super::event_bus::publish_web_channel_event_in;
use super::super::runner::{runtime, scope_of, CancelNote};
use super::super::types::ChatRequestMetadata;
use crate::agent::queued_turn::StoredAttachmentInput;

/// Cancel the in-flight turn(s) for a thread.
///
/// When `request_id` is `Some`, the cancel is **scoped**: it only tears down the
/// primary turn if that exact request is still running (and only the matching
/// parallel turn), so a stale cancel for a superseded request can't kill the
/// newer turn that replaced it (#4760). When `request_id` is `None`, it stops
/// whatever is running on the thread (primary + every parallel) — the "stop
/// everything" behaviour used by session teardown / a Stop button.
pub async fn cancel_chat_scoped(
    ctx: &DispatchCtx,
    client_id: &str,
    thread_id: &str,
    request_id: Option<&str>,
) -> Result<Option<String>, String> {
    Ok(
        cancel_chat_inner(ctx, client_id, thread_id, request_id, false)
            .await?
            .request_id,
    )
}

/// The placeholder `client_id` a lease-loss teardown logs under. It carries no
/// meaning: fence state is the explicit `fenced` flag of `cancel_chat_inner`.
const FENCE_CLIENT_ID: &str = "profile-fence";

/// Stop every turn running in the calling context — what a profile whose
/// lease was lost does before it closes. Each thread is torn down as an
/// unscoped stop would (primary, parallel turns and detached sub-agents).
/// Returns how many threads had something to stop.
pub async fn cancel_all_turns(ctx: &DispatchCtx) -> usize {
    let threads = super::state::live_thread_ids(ctx).await;
    let mut stopped = 0;
    for thread_id in &threads {
        match cancel_chat_inner(ctx, FENCE_CLIENT_ID, thread_id, None, true).await {
            Ok(outcome) if outcome.request_id.is_some() => stopped += 1,
            Ok(_) => {}
            Err(error) => {
                log::warn!("[web-channel] stopping thread_id={thread_id} failed: {error}");
            }
        }
    }
    log::info!(
        "[web-channel] cancel_all_turns threads={} stopped={stopped}",
        threads.len()
    );
    stopped
}

/// What one cancel tore down.
struct CancelOutcome {
    /// The primary or parallel turn that was cancelled, if any.
    request_id: Option<String>,
    /// Detached sub-agents stopped along with the turn (unscoped stops only).
    subagents_cancelled: usize,
}

async fn cancel_chat_inner(
    ctx: &DispatchCtx,
    client_id: &str,
    thread_id: &str,
    request_id: Option<&str>,
    // True for a lease-loss teardown no client asked for: each turn's events
    // then go to the client that started it instead of `client_id`.
    fenced: bool,
) -> Result<CancelOutcome, String> {
    let client_id = client_id.trim();
    let thread_id = thread_id.trim();

    if client_id.is_empty() {
        return Err("client_id is required".to_string());
    }
    if thread_id.is_empty() {
        return Err("thread_id is required".to_string());
    }

    let rt = runtime(ctx);
    let scope = scope_of(ctx);
    let live: Vec<_> = rt
        .runner
        .live_turns()
        .await
        .into_iter()
        .filter(|turn| turn.scope == scope && turn.thread_id == thread_id)
        .collect();
    // The runner publishes each cancelled turn's terminal event; tell it why.
    // A fence cancel names no real client, so each turn's events go to the
    // client that started it; a user's cancel keeps the caller's id.
    for turn in live
        .iter()
        .filter(|turn| request_id.is_none_or(|rid| turn.request_id == rid))
    {
        rt.shared.note_cancel(
            &turn.request_id,
            CancelNote {
                user_stop: true,
                client_id: (!fenced).then(|| client_id.to_string()),
                superseded_by: None,
            },
        );
    }
    // #4760: a scoped cancel tears down only the turn it names. A stale cancel
    // for an already-superseded request is a no-op, so the newer turn lives.
    let cancelled = rt
        .runner
        .cancel_requests(&scope, thread_id, request_id)
        .await;
    if cancelled.is_empty() {
        if let Some(rid) = request_id {
            log::info!(
                "[web-channel] ignoring stale cancel request_id={rid} for thread_id={thread_id} — current in-flight is {:?}; newer turn preserved",
                live.iter()
                    .find(|turn| !turn.parallel)
                    .map(|turn| turn.request_id.as_str())
            );
        }
    }

    // #4760: a scoped cancel that matched only a parallel (forked) turn still
    // genuinely tore a turn down; the primary comes first when both did.
    let cancelled_any = cancelled.first().cloned();

    // An unscoped stop also halts the thread's detached work. Async sub-agents
    // (`spawn_subagent`) run on their own tasks and deliberately drop the
    // spawning turn's cancellation, so tearing the turn down leaves them
    // running — and when one finishes, background delivery starts a fresh
    // system turn on the thread, which reads as "Stop did nothing". Abort them
    // first, then drop anything already queued for delivery, so no result lands
    // in the gap. A scoped cancel names one turn and leaves the rest alone.
    // A turn cancelled cooperatively may never publish a terminal event, which
    // would leave its session marked busy and defer this thread's background
    // deliveries until restart.
    //
    // Only once nothing is left running on the thread: a scoped cancel can leave
    // the primary or a parallel turn alive, and those still owe a terminal event.
    let others_live = live
        .iter()
        .any(|turn| !cancelled.contains(&turn.request_id));
    if cancelled_any.is_some() && !others_live {
        crate::agent::orchestration::busy_guard::clear_busy_for_thread(ctx, thread_id);
        // A drain that fired while the cancelled turn counted as busy returned
        // without claiming anything; ask for another.
        crate::agent::orchestration::background_delivery::kick_delivery(ctx, thread_id);
    }
    let subagents_cancelled = if request_id.is_none() {
        // Gate completion recording before aborting: Tokio abort is
        // cooperative, so a child already finishing can otherwise enqueue in
        // the gap between abort and the queue sweep.
        let discarded =
            crate::agent::orchestration::background_completions::discard_pending_for_thread(
                ctx, thread_id,
            );
        let stopped =
            crate::agent::orchestration::running_subagents::stop_for_thread(ctx, thread_id);
        crate::agent::orchestration::background_completions::finish_stop_for_thread(
            ctx, thread_id, &stopped,
        );
        log::info!(
            "[web-channel] stop thread_id={} turns={:?} subagents_cancelled={} completions_discarded={}",
            thread_id,
            cancelled,
            stopped.len(),
            discarded
        );
        stopped.len()
    } else {
        0
    };

    Ok(CancelOutcome {
        request_id: cancelled_any,
        subagents_cancelled,
    })
}

pub async fn channel_web_chat(
    ctx: &crate::core::runtime::DispatchCtx,
    client_id: &str,
    thread_id: &str,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    locale: Option<String>,
    queue_mode: Option<String>,
    run_mode: Option<String>,
    metadata: ChatRequestMetadata,
    voice_note_id: Option<String>,
) -> Result<Outcome<Value>, String> {
    channel_web_chat_with_stored_attachments(
        ctx,
        client_id,
        thread_id,
        message,
        model_override,
        temperature,
        locale,
        queue_mode,
        run_mode,
        metadata,
        voice_note_id,
        None,
    )
    .await
}

pub async fn channel_web_chat_with_stored_attachments(
    ctx: &crate::core::runtime::DispatchCtx,
    client_id: &str,
    thread_id: &str,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    locale: Option<String>,
    queue_mode: Option<String>,
    run_mode: Option<String>,
    metadata: ChatRequestMetadata,
    voice_note_id: Option<String>,
    stored_attachments: Option<StoredAttachmentInput>,
) -> Result<Outcome<Value>, String> {
    channel_web_chat_with_attachments(
        ctx,
        client_id,
        thread_id,
        message,
        model_override,
        temperature,
        locale,
        queue_mode,
        run_mode,
        metadata,
        voice_note_id,
        stored_attachments,
        None,
    )
    .await
}

pub async fn channel_web_chat_with_attachments(
    ctx: &crate::core::runtime::DispatchCtx,
    client_id: &str,
    thread_id: &str,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    locale: Option<String>,
    queue_mode: Option<String>,
    run_mode: Option<String>,
    metadata: ChatRequestMetadata,
    voice_note_id: Option<String>,
    stored_attachments: Option<StoredAttachmentInput>,
    attachment_uploads: Option<Vec<tinychannels_bus::ContentPart>>,
) -> Result<Outcome<Value>, String> {
    // A SaaS user chooses the thread id; the same rules as `threads_upsert`
    // apply (no reserved prefixes, no path-like ids). No-op outside SaaS.
    crate::profiles::surface::check_thread_id(thread_id.trim())?;
    // Mirrors the socket `chat:start` payload's `run_mode` handling
    // (`openhuman_rpc::server::socketio`): apply it before starting the turn so
    // `plan_mode_middleware` sees the requested mode from the first tool
    // check of this turn, rather than racing a separate
    // `agent.set_run_mode` RPC. Unrecognized values are logged and ignored
    // — a stale/typo'd client build must not fail the whole turn.
    if let Some(run_mode) = run_mode.as_deref() {
        match crate::threads::run_mode::parse_mode_label(run_mode) {
            Some(mode) => {
                crate::threads::run_mode::set_mode(ctx, thread_id, mode);
            }
            None => log::warn!(
                "[web_chat] channel_web_chat thread_id={thread_id} ignoring unrecognized run_mode={run_mode}"
            ),
        }
    }

    let result = super::start_chat::start_chat_with_attachments(
        ctx,
        client_id,
        thread_id,
        message,
        model_override,
        temperature,
        locale,
        queue_mode,
        metadata,
        voice_note_id,
        stored_attachments,
        attachment_uploads,
    )
    .await?;

    if let Ok(parsed) = serde_json::from_str::<Value>(&result) {
        return Ok(Outcome::single_log(parsed, "web channel message queued"));
    }

    let accepted = serde_json::to_value(crate::threads::rpc_models::StartedChatAck {
        accepted: true,
        client_id: client_id.trim().to_owned(),
        thread_id: thread_id.trim().to_owned(),
        request_id: result,
    })
    .map_err(|error| format!("serialize started chat ack: {error}"))?;
    Ok(Outcome::single_log(
        accepted,
        "web channel request accepted",
    ))
}

/// Render one snapshotted queue item as the wire shape `web_queue_status` and
/// `queue_item_*` socket events share: `{ id, lane, text_preview }`.
fn queue_item_json(
    lane: tinyagents_harness::run_queue::QueueLane,
    item: &crate::agent::queued_turn::QueuedTurn,
) -> Value {
    json!({
        "id": item.request_id,
        "lane": lane.as_str(),
        "text_preview": crate::agent::queued_turn::text_preview(&item.text),
    })
}

pub async fn channel_web_queue_status(
    ctx: &DispatchCtx,
    thread_id: &str,
) -> Result<Outcome<Value>, String> {
    let runner = &runtime(ctx).runner;
    let scope = scope_of(ctx);
    let Some(request_id) = runner.in_flight_request_id(&scope, thread_id).await else {
        return Ok(Outcome::single_log(
            json!({
                "thread_id": thread_id.trim(),
                "active": false,
                "steers": 0,
                "followups": 0,
                "collects": 0,
                "total": 0,
                "items": Vec::<Value>::new(),
            }),
            "no active turn for thread",
        ));
    };
    let status = runner.queue_status(&scope, thread_id).await;
    let items: Vec<Value> = runner
        .queue_snapshot(&scope, thread_id)
        .await
        .iter()
        .map(|(lane, item)| queue_item_json(*lane, item))
        .collect();
    Ok(Outcome::single_log(
        json!({
            "thread_id": thread_id.trim(),
            "active": true,
            "request_id": request_id,
            "steers": status.steers,
            "followups": status.followups,
            "collects": status.collects,
            "total": status.total,
            "items": items,
        }),
        "queue status retrieved",
    ))
}

/// `channel.web_queue_remove` — retract one specific queued item (e.g. the
/// user deleted a queued message from the composer's queue UI) without
/// touching the rest of the queue. Emits `queue_item_removed` on an actual
/// removal; a no-op removal (unknown id, or no active turn) is silently
/// `removed: false` — the item is already gone either way.
pub async fn channel_web_queue_remove(
    ctx: &DispatchCtx,
    client_id: &str,
    thread_id: &str,
    item_id: &str,
) -> Result<Outcome<Value>, String> {
    let client_id = client_id.trim();
    let thread_id = thread_id.trim();
    let item_id = item_id.trim();
    if item_id.is_empty() {
        return Err("item_id is required".to_string());
    }
    let rt = runtime(ctx);
    let scope = scope_of(ctx);
    if rt
        .runner
        .in_flight_request_id(&scope, thread_id)
        .await
        .is_none()
    {
        return Ok(Outcome::single_log(
            json!({
                "thread_id": thread_id,
                "item_id": item_id,
                "removed": false,
            }),
            "no active turn for thread",
        ));
    }
    let removed = rt.runner.remove_queued(&scope, thread_id, item_id).await;
    if removed > 0 {
        rt.shared.forget_queued(item_id);
        log::info!("[web-channel] removed queued item thread_id={thread_id} item_id={item_id}");
        publish_web_channel_event_in(
            ctx,
            WebChannelEvent::QueueItemRemoved {
                context: event_context(client_id, thread_id, ""),
                queue_item: Some(crate::web_chat::QueueItemPayload {
                    id: item_id.to_string(),
                    lane: None,
                    text_preview: None,
                }),
            },
        );
    }
    Ok(Outcome::single_log(
        json!({
            "thread_id": thread_id,
            "item_id": item_id,
            "removed": removed > 0,
        }),
        "queue item remove processed",
    ))
}

pub async fn channel_web_queue_clear(
    ctx: &DispatchCtx,
    thread_id: &str,
) -> Result<Outcome<Value>, String> {
    let rt = runtime(ctx);
    let scope = scope_of(ctx);
    if rt
        .runner
        .in_flight_request_id(&scope, thread_id)
        .await
        .is_none()
    {
        return Ok(Outcome::single_log(
            json!({
                "thread_id": thread_id.trim(),
                "cleared": false,
                "dropped": 0,
            }),
            "no active turn for thread",
        ));
    }
    for (_, item) in rt.runner.queue_snapshot(&scope, thread_id).await {
        rt.shared.forget_queued(&item.request_id);
    }
    let dropped = rt.runner.clear_queue(&scope, thread_id).await;
    log::info!("[web-channel] cleared queue thread_id={thread_id} dropped={dropped}");
    Ok(Outcome::single_log(
        json!({
            "thread_id": thread_id.trim(),
            "cleared": true,
            "dropped": dropped,
        }),
        "queue cleared",
    ))
}

pub async fn channel_web_cancel(
    ctx: &DispatchCtx,
    client_id: &str,
    thread_id: &str,
    request_id: Option<&str>,
) -> Result<Outcome<Value>, String> {
    let outcome = cancel_chat_inner(ctx, client_id, thread_id, request_id, false).await?;

    // `request_id` is set only when a turn was torn down, and only then does a
    // `cancelled` chat_error follow. A client that sees `request_id: null` knows
    // no terminal event is coming and must settle its own running state.
    let cancelled = outcome.request_id.is_some() || outcome.subagents_cancelled > 0;

    Ok(Outcome::single_log(
        json!({
            "cancelled": cancelled,
            "client_id": client_id.trim(),
            "thread_id": thread_id.trim(),
            "request_id": outcome.request_id,
            "subagents_cancelled": outcome.subagents_cancelled,
        }),
        "web channel cancellation processed",
    ))
}
