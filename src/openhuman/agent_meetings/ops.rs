//! RPC handlers for the `agent_meetings` domain.
//!
//! Each handler emits a Socket.IO event to the backend via the global
//! `SocketManager`. The backend's meeting bot handler picks these up and
//! drives the Recall.ai (or Camoufox) session.

use serde_json::{json, Map, Value};

use crate::core::event_bus::BackendMeetTurn;
use crate::openhuman::meet::ops::validate_display_name;
use crate::openhuman::memory::ingest_pipeline;
use crate::openhuman::memory_sync::canonicalize::chat::{ChatBatch, ChatMessage};
use crate::openhuman::socket::global_socket_manager;
use crate::rpc::RpcOutcome;

use super::types::{
    BackendMeetHarnessResponseRequest, BackendMeetJoinRequest, BackendMeetJoinResponse,
    BackendMeetLeaveRequest, BackendMeetSpeakRequest, MeetingSessionStatus,
};

const ALLOWED_HOSTS: &[(&str, &str)] = &[
    ("meet.google.com", "gmeet"),
    ("zoom.us", "zoom"),
    ("teams.microsoft.com", "teams"),
    ("webex.com", "webex"),
];

fn transcript_turns_to_chat_batch(
    turns: &[BackendMeetTurn],
    duration_ms: u64,
) -> Option<ChatBatch> {
    // Cap at 48 h to avoid DateTime underflow; real meetings never exceed this.
    const MAX_DURATION_MS: u64 = 172_800_000;
    let duration_i64 = i64::try_from(duration_ms.min(MAX_DURATION_MS)).unwrap_or(172_800_000);
    let base = chrono::Utc::now() - chrono::Duration::milliseconds(duration_i64);
    // Spread turns evenly across the duration; fall back to 1 ms spacing when
    // duration is zero or turns is empty (avoids division by zero).
    let spacing_ms = if turns.is_empty() {
        1i64
    } else {
        i64::try_from(duration_ms / turns.len() as u64).unwrap_or(1)
    };
    let mut messages = Vec::new();

    for (idx, turn) in turns.iter().enumerate() {
        let text = turn.content.trim();
        if text.is_empty() {
            continue;
        }
        let author = if turn.role.eq_ignore_ascii_case("assistant") {
            "Tiny"
        } else {
            "Meeting participant"
        };
        let offset_ms = spacing_ms.saturating_mul(idx as i64);
        messages.push(ChatMessage {
            author: author.to_string(),
            timestamp: base + chrono::Duration::milliseconds(offset_ms),
            text: text.to_string(),
            source_ref: Some(format!("backend-meet://turn/{idx}")),
        });
    }

    if messages.is_empty() {
        None
    } else {
        Some(ChatBatch {
            platform: "backend_meet".to_string(),
            channel_label: "Recall AI meeting".to_string(),
            messages,
        })
    }
}

pub async fn ingest_backend_meeting_transcript(
    turns: Vec<BackendMeetTurn>,
    duration_ms: u64,
    correlation_id: Option<String>,
) -> Result<(), String> {
    let Some(batch) = transcript_turns_to_chat_batch(&turns, duration_ms) else {
        tracing::debug!("[agent_meetings] transcript had no ingestible turns");
        return Ok(());
    };

    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| format!("[agent_meetings] config load failed: {e}"))?;
    let cid_suffix = correlation_id.as_deref().unwrap_or("none");
    let source_id = format!(
        "meet:recall:{}:{}",
        chrono::Utc::now().timestamp_millis(),
        cid_suffix
    );
    let tags = vec!["meeting".to_string(), "recall_ai".to_string()];
    let result = ingest_pipeline::ingest_chat(&config, &source_id, "user", tags, batch)
        .await
        .map_err(|e| format!("[agent_meetings] transcript ingest failed: {e:#}"))?;

    tracing::info!(
        source_id = %source_id,
        chunks_written = result.chunks_written,
        correlation_id = ?correlation_id,
        "[agent_meetings] transcript ingested into memory tree"
    );

    // Create a meeting thread with the transcript for the thread system. This
    // path has no pre-generated summary, so the thread generates its own.
    if let Err(e) =
        create_meeting_thread_with_transcript(&turns, duration_ms, correlation_id, None).await
    {
        tracing::warn!("[agent_meetings] meeting thread creation failed: {e}");
    }

    Ok(())
}

/// Create a conversation thread labelled "Meetings" containing the transcript.
///
/// The correlation_id (when present) is embedded in the transcript body as an
/// external reference for tracing — it does not deduplicate; each call creates
/// a new thread.
/// `generated` lets the caller pass a summary it already produced so the
/// call-end pipeline can share one generation across the recent-call detail
/// store and this thread. When `None`, the summary is generated here (bounded)
/// — preserving the behaviour of callers that don't pre-generate.
pub async fn create_meeting_thread_with_transcript(
    turns: &[BackendMeetTurn],
    duration_ms: u64,
    correlation_id: Option<String>,
    generated: Option<&super::summary::GeneratedSummary>,
) -> Result<(), String> {
    use crate::openhuman::memory::{
        AppendConversationMessageRequest, ConversationMessageRecord,
        CreateConversationThreadRequest, UpdateConversationThreadTitleRequest,
    };
    use crate::openhuman::threads::ops;

    if turns.is_empty() {
        return Ok(());
    }

    // Format the transcript body first — this is the durable artifact and must
    // not depend on (or wait on) the summarisation LLM call.
    let mut body = String::new();
    let duration_min = duration_ms / 60_000;
    body.push_str(&format!("Duration: {duration_min} min\n\n"));
    if let Some(cid) = &correlation_id {
        body.push_str(&format!("Correlation ID: {cid}\n\n"));
    }
    for turn in turns {
        let text = turn.content.trim();
        if text.is_empty() {
            continue;
        }
        let role_label = if turn.role.eq_ignore_ascii_case("assistant") {
            "Assistant"
        } else {
            "Participant"
        };
        body.push_str(&format!("**{role_label}**: {text}\n\n"));
    }

    // 1. Create the thread under the shared "Meetings" label and append the
    //    transcript *before* any LLM work, so thread/transcript persistence (and
    //    the memory-tree ingest that runs after this returns) never gate on
    //    summarisation. The per-meeting topic is applied later as the thread
    //    *title* only — adding it as a second label would accrue a unique,
    //    never-reused label per call and pollute the shared label taxonomy,
    //    while the title already disambiguates calls in the list.
    let create_req = CreateConversationThreadRequest {
        labels: Some(vec!["Meetings".to_string()]),
        personality_id: None,
    };
    let outcome = ops::thread_create_new(create_req)
        .await
        .map_err(|e| format!("[agent_meetings] thread creation failed: {e}"))?;
    let thread_id = outcome
        .value
        .data
        .as_ref()
        .ok_or_else(|| "[agent_meetings] thread creation returned no data".to_string())?
        .id
        .clone();

    // 2. Append the transcript as a message. The durable record is now complete
    //    regardless of whether summarisation succeeds below.
    let msg = ConversationMessageRecord {
        id: uuid::Uuid::new_v4().to_string(),
        content: body,
        message_type: "system".to_string(),
        extra_metadata: serde_json::Value::Null,
        sender: "system".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    let append_req = AppendConversationMessageRequest {
        thread_id: thread_id.clone(),
        message: msg,
    };
    if let Err(e) = ops::message_append(append_req).await {
        tracing::warn!(
            thread_id = %thread_id,
            "[agent_meetings] failed to append transcript message: {e}"
        );
    }

    // 3. Best-effort enrichment: reuse a summary the caller already generated
    //    (the call-end pipeline shares one across the recent-call detail store
    //    and this thread); otherwise generate one here, bounded so a slow/flaky
    //    provider can never dominate the path. Any failure or timeout leaves the
    //    plain-transcript thread untouched.
    let owned_generated = if generated.is_none() {
        super::summary::generate_meeting_summary_bounded(turns, correlation_id.as_deref()).await
    } else {
        None
    };
    let generated = generated.or(owned_generated.as_ref());

    // 3a. Title the thread with the context label (e.g. "Q3 Roadmap") so the
    //     meeting is identifiable in the list (default title is "Chat <date>").
    let context_label = generated.map(|g| g.label.trim()).filter(|l| !l.is_empty());
    if let Some(title) = context_label {
        if let Err(e) = ops::thread_update_title(UpdateConversationThreadTitleRequest {
            thread_id: thread_id.clone(),
            title: title.to_string(),
        })
        .await
        {
            tracing::warn!(
                thread_id = %thread_id,
                "[agent_meetings] failed to set meeting thread title: {e}"
            );
        }
    }

    // 3b. Append the structured summary as a closing message, so the thread ends
    //     with the headline / key points / action items.
    if let Some(g) = generated {
        let summary_body = super::summary::format_summary_markdown(&g.summary, &g.label);
        let summary_msg = ConversationMessageRecord {
            id: uuid::Uuid::new_v4().to_string(),
            content: summary_body,
            message_type: "system".to_string(),
            extra_metadata: serde_json::Value::Null,
            sender: "system".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let summary_req = AppendConversationMessageRequest {
            thread_id: thread_id.clone(),
            message: summary_msg,
        };
        if let Err(e) = ops::message_append(summary_req).await {
            tracing::warn!(
                thread_id = %thread_id,
                "[agent_meetings] failed to append summary message: {e}"
            );
        }
    }

    tracing::info!(
        thread_id = %thread_id,
        turn_count = turns.len(),
        summarized = generated.is_some(),
        "[agent_meetings] meeting thread created"
    );
    Ok(())
}

fn validate_meeting_url(raw: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(raw.trim()).map_err(|e| format!("invalid meeting URL: {e}"))?;

    if url.scheme() != "https" && url.scheme() != "http" {
        return Err(format!(
            "invalid meeting URL: scheme `{}` not allowed",
            url.scheme()
        ));
    }

    let host = url
        .host_str()
        .ok_or_else(|| "invalid meeting URL: missing host".to_string())?;

    let is_allowed = ALLOWED_HOSTS
        .iter()
        .any(|(allowed, _)| host == *allowed || host.ends_with(&format!(".{allowed}")));

    if !is_allowed {
        return Err(format!(
            "invalid meeting URL: host `{host}` not recognized (supported: Google Meet, Zoom, Teams, Webex)"
        ));
    }

    Ok(url)
}

fn infer_platform(url: &url::Url) -> &'static str {
    let host = url.host_str().unwrap_or("");
    for (allowed, platform) in ALLOWED_HOSTS {
        if host == *allowed || host.ends_with(&format!(".{allowed}")) {
            return platform;
        }
    }
    "gmeet"
}

/// Build the `bot:join` Socket.IO payload from a validated request.
///
/// Extracted as a pure function so it can be unit-tested independently of the
/// live socket connection.
fn build_join_payload(
    meet_url: &str,
    display_name: &str,
    platform: &str,
    req: &BackendMeetJoinRequest,
) -> Value {
    let mut payload = json!({
        "meetUrl": meet_url,
        "displayName": display_name,
        "platform": platform,
    });
    if let Some(map) = payload.as_object_mut() {
        if let Some(agent_name) = &req.agent_name {
            map.insert("agentName".to_string(), json!(agent_name));
        }
        if let Some(system_prompt) = &req.system_prompt {
            map.insert("systemPrompt".to_string(), json!(system_prompt));
        }
        if let Some(mascot_id) = &req.mascot_id {
            map.insert("mascotId".to_string(), json!(mascot_id));
        }
        if let Some(rive_colors) = &req.rive_colors {
            map.insert(
                "riveColors".to_string(),
                json!({
                    "primaryColor": rive_colors.primary_color,
                    "secondaryColor": rive_colors.secondary_color,
                }),
            );
        }
        if let Some(respond_to) = &req.respond_to_participant {
            map.insert("respondToParticipant".to_string(), json!(respond_to));
        }
        if let Some(phrase) = &req.wake_phrase {
            map.insert("wakePhrase".to_string(), json!(phrase));
        }
        if let Some(cid) = &req.correlation_id {
            map.insert("correlationId".to_string(), json!(cid));
        }
        if let Some(lo) = req.listen_only {
            map.insert("listenOnly".to_string(), json!(lo));
        }
    }
    payload
}

/// Pure: extract the reply anchor (`respondToParticipant`) carried by a
/// notification action payload. Returns `None` when absent or blank.
fn anchor_from_action_payload(payload: &Value) -> Option<String> {
    payload
        .get("respondToParticipant")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Pure: build the `agent_meetings_join` param map for an AskEachTime
/// notification action.
///
/// Encapsulates the listen-only decision (reply mode without a known anchor is
/// downgraded to listen-only via [`super::calendar::effective_listen_only`]),
/// the wake phrase, and the `respond_to_participant` anchor wiring so they are
/// unit-testable without a live socket/config.
fn build_notification_join_map(
    action_id: &str,
    meet_url: &str,
    correlation_id: &str,
    display_name: Option<&str>,
    respond_to_participant: Option<&str>,
    config_listen_only_default: bool,
) -> Map<String, Value> {
    let requested_listen_only = match action_id {
        "join_listen" => true,
        "join_active" => false,
        _ => config_listen_only_default,
    };
    // Reply mode needs a known anchor. Without one, downgrade to listen-only
    // (still transcribes + summarizes) instead of replying to every speaker.
    let listen_only = super::calendar::effective_listen_only(
        requested_listen_only,
        respond_to_participant.is_some(),
    );
    if listen_only && !requested_listen_only {
        tracing::warn!(
            action_id = %action_id,
            "[agent_meetings] no reply anchor resolved — forcing listen-only join"
        );
    }

    let mut join = Map::new();
    join.insert("meet_url".to_string(), json!(meet_url));
    join.insert("correlation_id".to_string(), json!(correlation_id));
    join.insert("listen_only".to_string(), json!(listen_only));
    if let Some(name) = display_name {
        join.insert("display_name".to_string(), json!(name));
    }
    if !listen_only {
        // Reply mode: the participant addresses the bot as "Hey Tiny"; the
        // wake phrase is always required (no implicit address).
        join.insert("wake_phrase".to_string(), json!("Hey Tiny"));
        // Anchor replies to the meeting owner so the bot knows who it is
        // answering (empty/absent = respond to everyone).
        if let Some(owner) = respond_to_participant {
            join.insert("respond_to_participant".to_string(), json!(owner));
        }
    }
    join
}

/// Handle `openhuman.agent_meetings_join`.
pub async fn handle_join(params: Map<String, Value>) -> Result<Value, String> {
    let req: BackendMeetJoinRequest = serde_json::from_value(Value::Object(params))
        .map_err(|e| format!("[agent_meetings] invalid join params: {e}"))?;

    let normalized_url =
        validate_meeting_url(&req.meet_url).map_err(|e| format!("[agent_meetings] {e}"))?;

    let display_name = match &req.display_name {
        Some(name) => validate_display_name(name).map_err(|e| format!("[agent_meetings] {e}"))?,
        None => "Tiny".to_string(),
    };

    let inferred = infer_platform(&normalized_url);
    let platform = match req.platform.as_deref() {
        Some(p) if p != inferred => {
            return Err(format!(
                "[agent_meetings] platform mismatch: URL implies `{inferred}` but `{p}` was supplied"
            ));
        }
        Some(p) => p,
        None => inferred,
    };

    let mgr = global_socket_manager()
        .ok_or_else(|| "[agent_meetings] socket not connected to backend".to_string())?;

    if !mgr.is_connected() {
        return Err("[agent_meetings] socket not connected to backend".to_string());
    }

    tracing::info!(
        meet_url_host = %normalized_url.host_str().unwrap_or(""),
        platform = %platform,
        display_name_len = display_name.len(),
        "[agent_meetings] emitting bot:join"
    );

    let join_payload = build_join_payload(normalized_url.as_str(), &display_name, platform, &req);

    mgr.emit("bot:join", join_payload)
        .await
        .map_err(|e| format!("[agent_meetings] emit failed: {e}"))?;

    // Snapshot join context so the post-call recent-calls record can show who
    // launched the bot, into which meeting. Keyed by correlation_id; consumed
    // when the `BackendMeetTranscript` event arrives at call-end. No-op when
    // the caller didn't supply a correlation_id.
    super::recent_calls::remember_join(
        req.correlation_id.as_deref(),
        super::recent_calls::JoinMeta {
            meet_url: normalized_url.to_string(),
            // "Your Name in This Meeting" — the human who launched the bot and
            // whom it answers to. This is the owner shown in the recent-calls list.
            owner_display_name: req.respond_to_participant.clone().unwrap_or_default(),
            // The bot's tile name in the meeting (persona display name).
            bot_display_name: display_name.clone(),
            started_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        },
    );

    // Active mode (listen_only = false, the modal's "respond when addressed"
    // toggle) enables in-call agency for just this meeting, so the toggle
    // "just works" without flipping the global config. Passive joins leave
    // the meeting unmarked (default: listen-only / transcribe-only).
    if req.listen_only == Some(false) {
        super::in_call::mark_meeting_active(req.correlation_id.as_deref()).await;
    }

    let response = BackendMeetJoinResponse {
        ok: true,
        meet_url: normalized_url.to_string(),
        platform: platform.to_string(),
    };
    let outcome = RpcOutcome::new(
        serde_json::to_value(response).map_err(|e| format!("[agent_meetings] serialize: {e}"))?,
        vec![],
    );
    outcome.into_cli_compatible_json()
}

/// Handle `openhuman.agent_meetings_leave`.
pub async fn handle_leave(params: Map<String, Value>) -> Result<Value, String> {
    let req: BackendMeetLeaveRequest = serde_json::from_value(Value::Object(params))
        .map_err(|e| format!("[agent_meetings] invalid leave params: {e}"))?;

    let mgr = global_socket_manager()
        .ok_or_else(|| "[agent_meetings] socket not connected to backend".to_string())?;

    if !mgr.is_connected() {
        return Err("[agent_meetings] socket not connected to backend".to_string());
    }

    let reason = req.reason.unwrap_or_else(|| "requested".to_string());

    tracing::info!(reason = %reason, "[agent_meetings] emitting bot:leave");

    mgr.emit("bot:leave", json!({ "reason": reason }))
        .await
        .map_err(|e| format!("[agent_meetings] emit failed: {e}"))?;

    let outcome = RpcOutcome::new(json!({ "ok": true }), vec![]);
    outcome.into_cli_compatible_json()
}

/// Handle `openhuman.agent_meetings_harness_response`.
pub async fn handle_harness_response(params: Map<String, Value>) -> Result<Value, String> {
    let req: BackendMeetHarnessResponseRequest = serde_json::from_value(Value::Object(params))
        .map_err(|e| format!("[agent_meetings] invalid harness_response params: {e}"))?;

    if req.result.trim().is_empty() {
        return Err("[agent_meetings] result must not be empty".to_string());
    }

    let mgr = global_socket_manager()
        .ok_or_else(|| "[agent_meetings] socket not connected to backend".to_string())?;

    if !mgr.is_connected() {
        return Err("[agent_meetings] socket not connected to backend".to_string());
    }

    tracing::info!(
        result_len = req.result.len(),
        "[agent_meetings] emitting bot:harness:response"
    );

    mgr.emit("bot:harness:response", json!({ "result": req.result }))
        .await
        .map_err(|e| format!("[agent_meetings] emit failed: {e}"))?;

    let outcome = RpcOutcome::new(json!({ "ok": true }), vec![]);
    outcome.into_cli_compatible_json()
}

/// Handle `openhuman.agent_meetings_speak`.
pub async fn handle_speak(params: Map<String, Value>) -> Result<Value, String> {
    let req: BackendMeetSpeakRequest = serde_json::from_value(Value::Object(params))
        .map_err(|e| format!("[agent_meetings] invalid speak request: {e}"))?;

    if req.text.trim().is_empty() {
        return Err("[agent_meetings] text must not be empty".to_string());
    }

    let mgr = global_socket_manager()
        .ok_or_else(|| "[agent_meetings] socket not connected to backend".to_string())?;

    if !mgr.is_connected() {
        return Err("[agent_meetings] socket not connected to backend".to_string());
    }

    tracing::info!(
        text_len = req.text.len(),
        correlation_id = ?req.correlation_id,
        "[agent_meetings] emitting bot:speak"
    );

    let mut speak_payload = json!({ "text": req.text });
    if let Some(map) = speak_payload.as_object_mut() {
        if let Some(cid) = &req.correlation_id {
            map.insert("correlationId".to_string(), json!(cid));
        }
    }

    mgr.emit("bot:speak", speak_payload)
        .await
        .map_err(|e| format!("[agent_meetings] emit failed: {e}"))?;

    let outcome = RpcOutcome::new(json!({ "ok": true }), vec![]);
    outcome.into_cli_compatible_json()
}

/// Handle `openhuman.agent_meetings_notification_action` — a click on one
/// of the calendar auto-join notification buttons (issue #3507).
///
/// Actions:
/// - `join_listen`  → join muted (transcript-only).
/// - `join_active`  → join in reply mode with the "Hey Tiny" wake phrase.
/// - `skip`         → mark the meeting session Ended; no join.
/// - `always_join`  → persist `auto_join_policy = Always`, then join with
///   the configured `listen_only_default`.
///
/// `payload` carries `{ meetingId, meetUrl, title }` from the notification
/// plus an optional user-edited `displayName`.
pub async fn handle_notification_action(params: Map<String, Value>) -> Result<Value, String> {
    let action_id = params
        .get("action_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if action_id.is_empty() {
        return Err("[agent_meetings] action_id is required".to_string());
    }
    let payload = params.get("payload").cloned().unwrap_or(Value::Null);
    let meeting_id = payload
        .get("meetingId")
        .and_then(|v| v.as_str())
        .map(String::from);
    let meet_url = payload
        .get("meetUrl")
        .and_then(|v| v.as_str())
        .map(String::from);
    let display_name = payload
        .get("displayName")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    // Reply anchor carried from the calendar notification (issue: gmeet
    // auto-join anchor). Falls back to the signed-in account identity so a
    // notification raised before the anchor wiring still knows who to reply to.
    let respond_to_participant = anchor_from_action_payload(&payload).or_else(|| {
        crate::openhuman::app_state::peek_cached_current_user_identity()
            .and_then(|i| i.name)
            .map(|n| n.trim().to_string())
            .filter(|s| !s.is_empty())
    });

    tracing::info!(
        action_id = %action_id,
        meeting_id = ?meeting_id,
        has_meet_url = meet_url.is_some(),
        "[agent_meetings] notification action received"
    );

    match action_id.as_str() {
        "skip" => {
            if let Some(id) = &meeting_id {
                match crate::openhuman::config::ops::load_config_with_timeout().await {
                    Ok(config) => {
                        let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
                        if let Err(e) = super::store::update_session_status(
                            &config,
                            id,
                            MeetingSessionStatus::Ended,
                            now_ms,
                        ) {
                            tracing::debug!("[agent_meetings] skip: session update failed: {e}");
                        }
                    }
                    Err(e) => {
                        tracing::debug!("[agent_meetings] skip: config load failed: {e}");
                    }
                }
            }
            let outcome = RpcOutcome::new(json!({ "ok": true }), vec![]);
            outcome.into_cli_compatible_json()
        }
        "join_listen" | "join_active" | "always_join" => {
            let meet_url = meet_url
                .ok_or_else(|| "[agent_meetings] payload.meetUrl is required".to_string())?;
            let config = crate::openhuman::config::ops::load_config_with_timeout().await?;

            if action_id == "always_join" {
                let mut cfg = config.clone();
                cfg.meet.auto_join_policy =
                    crate::openhuman::config::schema::AutoJoinPolicy::Always;
                if let Err(e) = cfg.save().await {
                    // Join anyway — the policy flip failing must not block
                    // the join the user just asked for.
                    tracing::warn!("[agent_meetings] persisting always-join policy failed: {e}");
                }
            }

            let correlation_id = meeting_id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let join = build_notification_join_map(
                &action_id,
                &meet_url,
                &correlation_id,
                display_name.as_deref(),
                respond_to_participant.as_deref(),
                config.meet.listen_only_default,
            );

            handle_join(join).await
        }
        other => Err(format!("[agent_meetings] unknown action_id: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_google_meet_url() {
        validate_meeting_url("https://meet.google.com/abc-defg-hij").unwrap();
    }

    #[test]
    fn accepts_zoom_url() {
        validate_meeting_url("https://zoom.us/j/123456789").unwrap();
        validate_meeting_url("https://company.zoom.us/j/123456789").unwrap();
    }

    #[test]
    fn accepts_teams_url() {
        validate_meeting_url("https://teams.microsoft.com/l/meetup-join/abc").unwrap();
    }

    #[test]
    fn accepts_webex_url() {
        validate_meeting_url("https://meet.webex.com/meet/abc").unwrap();
        validate_meeting_url("https://company.webex.com/meet/abc").unwrap();
    }

    #[test]
    fn rejects_unknown_host() {
        assert!(validate_meeting_url("https://example.com/meeting").is_err());
    }

    #[tokio::test]
    async fn notification_action_requires_action_id() {
        let err = handle_notification_action(Map::new()).await.unwrap_err();
        assert!(err.contains("action_id"));
    }

    #[tokio::test]
    async fn notification_action_rejects_unknown_action() {
        let mut params = Map::new();
        params.insert("action_id".to_string(), json!("explode"));
        let err = handle_notification_action(params).await.unwrap_err();
        assert!(err.contains("unknown action_id"));
    }

    // ── anchor_from_action_payload ──────────────────────────────

    #[test]
    fn anchor_extracted_from_payload() {
        let payload = json!({ "respondToParticipant": "Shanu Goyanka" });
        assert_eq!(
            anchor_from_action_payload(&payload).as_deref(),
            Some("Shanu Goyanka")
        );
    }

    #[test]
    fn anchor_none_when_absent_or_blank() {
        assert!(anchor_from_action_payload(&json!({})).is_none());
        assert!(anchor_from_action_payload(&json!({ "respondToParticipant": "  " })).is_none());
    }

    // ── build_notification_join_map ─────────────────────────────

    #[test]
    fn join_map_listen_only_action_has_no_anchor_or_wake() {
        let join = build_notification_join_map(
            "join_listen",
            "https://meet.google.com/abc",
            "corr-1",
            Some("Tiny"),
            Some("Shanu"),
            false,
        );
        assert_eq!(join["listen_only"], json!(true));
        assert_eq!(join["display_name"], json!("Tiny"));
        // listen-only never carries wake/anchor
        assert!(!join.contains_key("wake_phrase"));
        assert!(!join.contains_key("respond_to_participant"));
    }

    #[test]
    fn join_map_active_with_anchor_carries_wake_and_anchor() {
        let join = build_notification_join_map(
            "join_active",
            "https://meet.google.com/abc",
            "corr-1",
            None,
            Some("Shanu"),
            false,
        );
        assert_eq!(join["listen_only"], json!(false));
        assert_eq!(join["wake_phrase"], json!("Hey Tiny"));
        assert_eq!(join["respond_to_participant"], json!("Shanu"));
        assert!(!join.contains_key("display_name"));
    }

    #[test]
    fn join_map_active_without_anchor_downgrades_to_listen_only() {
        let join = build_notification_join_map(
            "join_active",
            "https://meet.google.com/abc",
            "corr-1",
            None,
            None,
            false,
        );
        // No anchor → forced listen-only, no wake/anchor emitted.
        assert_eq!(join["listen_only"], json!(true));
        assert!(!join.contains_key("wake_phrase"));
        assert!(!join.contains_key("respond_to_participant"));
    }

    #[test]
    fn join_map_always_join_uses_config_default() {
        // always_join + config default reply (false) + anchor → reply mode.
        let reply = build_notification_join_map(
            "always_join",
            "https://meet.google.com/abc",
            "corr-1",
            None,
            Some("Shanu"),
            false,
        );
        assert_eq!(reply["listen_only"], json!(false));
        assert_eq!(reply["respond_to_participant"], json!("Shanu"));

        // always_join + config default listen-only (true) → listen-only.
        let passive = build_notification_join_map(
            "always_join",
            "https://meet.google.com/abc",
            "corr-1",
            None,
            Some("Shanu"),
            true,
        );
        assert_eq!(passive["listen_only"], json!(true));
        assert!(!passive.contains_key("respond_to_participant"));
    }

    #[tokio::test]
    async fn notification_action_join_requires_meet_url() {
        let mut params = Map::new();
        params.insert("action_id".to_string(), json!("join_listen"));
        params.insert("payload".to_string(), json!({ "meetingId": "m-1" }));
        let err = handle_notification_action(params).await.unwrap_err();
        assert!(err.contains("meetUrl"));
    }

    #[tokio::test]
    async fn notification_action_skip_without_meeting_id_is_ok() {
        // No meetingId → nothing to update; must succeed without touching
        // config or the session store.
        let mut params = Map::new();
        params.insert("action_id".to_string(), json!("skip"));
        let value = handle_notification_action(params).await.unwrap();
        assert_eq!(value.get("ok"), Some(&json!(true)));
    }

    #[test]
    fn infers_platform_from_host() {
        let url = url::Url::parse("https://meet.google.com/abc-defg-hij").unwrap();
        assert_eq!(infer_platform(&url), "gmeet");

        let url = url::Url::parse("https://zoom.us/j/123").unwrap();
        assert_eq!(infer_platform(&url), "zoom");

        let url = url::Url::parse("https://teams.microsoft.com/l/meetup").unwrap();
        assert_eq!(infer_platform(&url), "teams");

        let url = url::Url::parse("https://meet.webex.com/meet/abc").unwrap();
        assert_eq!(infer_platform(&url), "webex");

        let url = url::Url::parse("https://company.zoom.us/j/123").unwrap();
        assert_eq!(infer_platform(&url), "zoom");
    }

    #[test]
    fn transcript_turns_convert_to_chat_batch() {
        let batch = transcript_turns_to_chat_batch(
            &[
                BackendMeetTurn {
                    role: "user".to_string(),
                    content: "[Alice] OpenHuman, summarize this.".to_string(),
                },
                BackendMeetTurn {
                    role: "assistant".to_string(),
                    content: "Sure, here is the summary.".to_string(),
                },
            ],
            1_000,
        )
        .expect("batch");

        assert_eq!(batch.platform, "backend_meet");
        assert_eq!(batch.messages.len(), 2);
        assert_eq!(batch.messages[0].author, "Meeting participant");
        assert_eq!(batch.messages[1].author, "Tiny");
        assert!(batch.messages[0].text.contains("summarize"));
    }

    #[tokio::test]
    async fn join_fails_when_socket_not_connected() {
        let params: Map<String, Value> =
            serde_json::from_value(json!({"meet_url": "https://meet.google.com/abc-defg-hij"}))
                .unwrap();
        let result = handle_join(params).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("socket not connected"));
    }

    #[tokio::test]
    async fn harness_response_rejects_empty_result() {
        let params: Map<String, Value> = serde_json::from_value(json!({"result": "   "})).unwrap();
        let result = handle_harness_response(params).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must not be empty"));
    }

    // --- build_join_payload ---

    fn minimal_req(meet_url: &str) -> BackendMeetJoinRequest {
        serde_json::from_value(json!({ "meet_url": meet_url })).unwrap()
    }

    #[test]
    fn build_join_payload_minimal() {
        let req = minimal_req("https://meet.google.com/abc-defg-hij");
        let payload = build_join_payload(
            "https://meet.google.com/abc-defg-hij",
            "OpenHuman",
            "gmeet",
            &req,
        );
        assert_eq!(payload["meetUrl"], "https://meet.google.com/abc-defg-hij");
        assert_eq!(payload["displayName"], "OpenHuman");
        assert_eq!(payload["platform"], "gmeet");
        assert!(payload.get("agentName").is_none());
        assert!(payload.get("systemPrompt").is_none());
        assert!(payload.get("mascotId").is_none());
        assert!(payload.get("riveColors").is_none());
        assert!(payload.get("respondToParticipant").is_none());
        assert!(payload.get("wakePhrase").is_none());
    }

    #[test]
    fn build_join_payload_with_respond_to_participant() {
        let req: BackendMeetJoinRequest = serde_json::from_value(json!({
            "meet_url": "https://zoom.us/j/123",
            "respond_to_participant": "Alice"
        }))
        .unwrap();
        let payload = build_join_payload("https://zoom.us/j/123", "Bot", "zoom", &req);
        assert_eq!(payload["respondToParticipant"], "Alice");
        assert!(payload.get("wakePhrase").is_none());
    }

    #[test]
    fn build_join_payload_with_wake_phrase() {
        let req: BackendMeetJoinRequest = serde_json::from_value(json!({
            "meet_url": "https://zoom.us/j/123",
            "wake_phrase": "Hey bot"
        }))
        .unwrap();
        let payload = build_join_payload("https://zoom.us/j/123", "Bot", "zoom", &req);
        assert_eq!(payload["wakePhrase"], "Hey bot");
        assert!(payload.get("respondToParticipant").is_none());
    }

    #[test]
    fn build_join_payload_with_all_optional_fields() {
        let req: BackendMeetJoinRequest = serde_json::from_value(json!({
            "meet_url": "https://teams.microsoft.com/l/meet/abc",
            "agent_name": "MyBot",
            "system_prompt": "You are a helpful assistant.",
            "mascot_id": "yellow",
            "rive_colors": {
                "primary_color": "#ff0000",
                "secondary_color": "#00ff00"
            },
            "respond_to_participant": "Bob",
            "wake_phrase": "Hello bot"
        }))
        .unwrap();
        let payload = build_join_payload(
            "https://teams.microsoft.com/l/meet/abc",
            "MyBot",
            "teams",
            &req,
        );
        assert_eq!(payload["agentName"], "MyBot");
        assert_eq!(payload["systemPrompt"], "You are a helpful assistant.");
        assert_eq!(payload["mascotId"], "yellow");
        assert_eq!(payload["riveColors"]["primaryColor"], "#ff0000");
        assert_eq!(payload["riveColors"]["secondaryColor"], "#00ff00");
        assert_eq!(payload["respondToParticipant"], "Bob");
        assert_eq!(payload["wakePhrase"], "Hello bot");
    }

    #[test]
    fn join_request_fields_deserialize_correctly() {
        let req: BackendMeetJoinRequest = serde_json::from_value(json!({
            "meet_url": "https://meet.google.com/abc-defg-hij",
            "respond_to_participant": "Alice",
            "wake_phrase": "Hey bot"
        }))
        .unwrap();
        assert_eq!(req.respond_to_participant.as_deref(), Some("Alice"));
        assert_eq!(req.wake_phrase.as_deref(), Some("Hey bot"));
    }

    #[test]
    fn join_request_optional_fields_absent_by_default() {
        let req: BackendMeetJoinRequest =
            serde_json::from_value(json!({ "meet_url": "https://meet.google.com/abc-defg-hij" }))
                .unwrap();
        assert!(req.respond_to_participant.is_none());
        assert!(req.wake_phrase.is_none());
        assert!(req.agent_name.is_none());
        assert!(req.system_prompt.is_none());
        assert!(req.mascot_id.is_none());
        assert!(req.rive_colors.is_none());
    }

    #[test]
    fn build_join_payload_with_correlation_id() {
        let req: BackendMeetJoinRequest = serde_json::from_value(json!({
            "meet_url": "https://meet.google.com/abc-defg-hij",
            "correlation_id": "meeting-123"
        }))
        .unwrap();
        let payload = build_join_payload(
            "https://meet.google.com/abc-defg-hij",
            "OpenHuman",
            "gmeet",
            &req,
        );
        assert_eq!(payload["correlationId"], "meeting-123");
    }

    #[test]
    fn build_join_payload_with_listen_only() {
        let req: BackendMeetJoinRequest = serde_json::from_value(json!({
            "meet_url": "https://meet.google.com/abc-defg-hij",
            "listen_only": true
        }))
        .unwrap();
        let payload = build_join_payload(
            "https://meet.google.com/abc-defg-hij",
            "OpenHuman",
            "gmeet",
            &req,
        );
        assert_eq!(payload["listenOnly"], true);
    }

    #[test]
    fn build_join_payload_correlation_and_listen_only_absent_by_default() {
        let req = minimal_req("https://meet.google.com/abc-defg-hij");
        let payload = build_join_payload(
            "https://meet.google.com/abc-defg-hij",
            "OpenHuman",
            "gmeet",
            &req,
        );
        assert!(payload.get("correlationId").is_none());
        assert!(payload.get("listenOnly").is_none());
    }

    #[test]
    fn join_request_correlation_and_listen_only_deserialize() {
        let req: BackendMeetJoinRequest = serde_json::from_value(json!({
            "meet_url": "https://meet.google.com/abc-defg-hij",
            "correlation_id": "sess-456",
            "listen_only": true
        }))
        .unwrap();
        assert_eq!(req.correlation_id.as_deref(), Some("sess-456"));
        assert_eq!(req.listen_only, Some(true));
    }

    #[test]
    fn transcript_turns_empty_returns_none() {
        let result = transcript_turns_to_chat_batch(&[], 1_000);
        assert!(result.is_none());
    }

    #[test]
    fn transcript_turns_all_blank_content_returns_none() {
        let result = transcript_turns_to_chat_batch(
            &[BackendMeetTurn {
                role: "user".to_string(),
                content: "   ".to_string(),
            }],
            1_000,
        );
        assert!(result.is_none());
    }

    #[test]
    fn transcript_turns_zero_duration_no_panic() {
        let batch = transcript_turns_to_chat_batch(
            &[BackendMeetTurn {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            0,
        )
        .expect("batch");
        assert_eq!(batch.messages.len(), 1);
    }

    #[test]
    fn rive_colors_deserialize() {
        use crate::openhuman::agent_meetings::types::RiveColors;
        let rc: RiveColors =
            serde_json::from_value(json!({"primary_color": "#abc", "secondary_color": "#def"}))
                .unwrap();
        assert_eq!(rc.primary_color.as_deref(), Some("#abc"));
        assert_eq!(rc.secondary_color.as_deref(), Some("#def"));
    }
}
