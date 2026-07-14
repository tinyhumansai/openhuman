//! Event-bus subscriber that reacts to backend meeting events.
//!
//! - `BackendMeetTranscript` → creates a dedicated "Meetings"-labelled
//!   conversation thread and appends the transcript.
//! - `BackendMeetJoined` / `BackendMeetLeft` → logged for audit trail;
//!   session status tracking is handled by the frontend Redux slice.

use std::sync::OnceLock;

use async_trait::async_trait;

use crate::core::event_bus::{DomainEvent, EventHandler, SubscriptionHandle};

use super::ops::{
    append_summary_prompt_message, create_meeting_thread_with_transcript_with_summary_mode,
    ingest_backend_meeting_transcript, SummaryGenerationMode,
};

static MEETING_EVENT_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

const LOG_PREFIX: &str = "[agent_meetings::bus]";

/// Register the meeting event subscriber. Idempotent — second+ calls are
/// no-ops.
pub fn register_meeting_event_subscriber() {
    if MEETING_EVENT_HANDLE.get().is_some() {
        return;
    }

    match crate::core::event_bus::subscribe_global(std::sync::Arc::new(MeetingEventSubscriber)) {
        Some(handle) => {
            let _ = MEETING_EVENT_HANDLE.set(handle);
            tracing::info!("{LOG_PREFIX} registered");
        }
        None => {
            tracing::warn!("{LOG_PREFIX} failed to register — event bus not initialized");
        }
    }
}

pub struct MeetingEventSubscriber;

#[async_trait]
impl EventHandler for MeetingEventSubscriber {
    fn name(&self) -> &str {
        "agent_meetings::events"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["agent_meetings"])
    }

    async fn handle(&self, event: &DomainEvent) {
        match event {
            DomainEvent::BackendMeetTranscript {
                turns,
                duration_ms,
                correlation_id,
            } => {
                tracing::info!(
                    turn_count = turns.len(),
                    duration_ms = duration_ms,
                    correlation_id = ?correlation_id,
                    "{LOG_PREFIX} transcript received — creating meeting thread"
                );

                // 1. Record a lean recent-calls entry (meet id, duration, owner,
                //    participants) first — before any LLM work — so the row is on
                //    disk by the time the panel refetches at call-end. Returns the
                //    request_id so the detail below is keyed to the same call.
                //    Best-effort: never blocks; logs on failure internally.
                let request_id = super::recent_calls::record_backend_call(
                    turns,
                    *duration_ms,
                    correlation_id.as_deref(),
                )
                .await;

                // 2. Persist the transcript immediately — decoupled from the
                //    (bounded, up to 30s) summary LLM call below. Without this the
                //    detail file wouldn't exist until summarisation returned, so a
                //    row expanded right after call-end showed "nothing captured"
                //    even though the transcript was already in hand. The summary is
                //    patched in by step 4 once it's ready.
                super::recent_calls::record_backend_call_detail(&request_id, turns, None).await;

                let (policy, summary_decision) =
                    match crate::openhuman::config::Config::load_or_init().await {
                        Ok(config) => {
                            let policy = config.meet.auto_summarize_policy;
                            (
                                Some(policy),
                                super::summary::post_call_summary_decision(policy),
                            )
                        }
                        Err(e) => {
                            tracing::warn!(
                                "{LOG_PREFIX} config load failed while resolving summary policy; skipping auto-summary: {e}"
                            );
                            (None, super::summary::PostCallSummaryDecision::Skip)
                        }
                    };
                tracing::info!(
                    policy = ?policy,
                    decision = ?summary_decision,
                    request_id = %request_id,
                    "{LOG_PREFIX} resolved post-call summary policy"
                );

                // 3. Generate the post-call summary only when the user chose
                //    Always. Ask/Never keep the transcript durable without
                //    spending an LLM call.
                let generated = if matches!(
                    summary_decision,
                    super::summary::PostCallSummaryDecision::Generate
                ) {
                    super::summary::generate_meeting_summary_bounded(
                        turns,
                        correlation_id.as_deref(),
                    )
                    .await
                } else {
                    None
                };

                // 4. Upgrade the stored detail with the summary once it's ready.
                //    Skipped when summarisation failed/timed out — the transcript
                //    written in step 2 stands on its own.
                if generated.is_some() {
                    super::recent_calls::record_backend_call_detail(
                        &request_id,
                        turns,
                        generated.as_ref(),
                    )
                    .await;
                }

                // 5. Create the meeting thread with transcript, reusing the
                //    summary generated in step 3.
                match create_meeting_thread_with_transcript_with_summary_mode(
                    turns,
                    *duration_ms,
                    correlation_id.clone(),
                    generated.as_ref(),
                    SummaryGenerationMode::UseProvidedOnly,
                )
                .await
                {
                    Ok(thread_id) => {
                        if matches!(
                            summary_decision,
                            super::summary::PostCallSummaryDecision::Prompt
                        ) {
                            if let Err(e) =
                                append_summary_prompt_message(&thread_id, &request_id).await
                            {
                                tracing::warn!("{LOG_PREFIX} summary prompt append failed: {e}");
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("{LOG_PREFIX} meeting thread creation failed: {e}");
                    }
                }

                // Also ingest into memory tree (existing pipeline).
                let enabled = crate::openhuman::config::Config::load_or_init()
                    .await
                    .map(|c| c.meet.ingest_backend_transcripts)
                    .unwrap_or(false);
                if enabled {
                    if let Err(e) = ingest_backend_meeting_transcript(
                        turns.clone(),
                        *duration_ms,
                        correlation_id.clone(),
                    )
                    .await
                    {
                        tracing::warn!("{LOG_PREFIX} memory ingest failed: {e}");
                    }
                } else {
                    tracing::debug!(
                        "{LOG_PREFIX} memory ingest skipped (config.meet.ingest_backend_transcripts = false)"
                    );
                }
            }

            DomainEvent::BackendMeetJoined {
                meet_url,
                correlation_id,
            } => {
                tracing::info!(
                    meet_url_len = meet_url.len(),
                    correlation_id = ?correlation_id,
                    "{LOG_PREFIX} bot joined meeting"
                );
                // Pre-warm the per-meeting orchestrator so the first
                // wake-phrase command doesn't pay the 5-10s cold build.
                // Spawned (the build is slow) and gated on agency being
                // enabled, so listen-only / agency-off meetings don't build
                // an agent they'll never use.
                let correlation_id = correlation_id.clone();
                tokio::spawn(async move {
                    let agency_on = crate::openhuman::config::Config::load_or_init()
                        .await
                        .map(|c| c.meet.enable_in_call_agency)
                        .unwrap_or(false);
                    // Also pre-warm for meetings joined in active mode via the
                    // per-meeting toggle, so they get the same first-command
                    // latency win as globally-enabled agency.
                    let active = super::in_call::is_meeting_active(correlation_id.as_deref()).await;
                    if agency_on || active {
                        super::in_call::prewarm_agent(correlation_id.as_deref()).await;
                    }
                });
            }

            DomainEvent::BackendMeetLeft {
                reason,
                correlation_id,
            } => {
                tracing::info!(
                    reason = %reason,
                    correlation_id = ?correlation_id,
                    "{LOG_PREFIX} bot left meeting"
                );
                // Free the per-meeting orchestrator built for in-call agency.
                super::in_call::clear_meeting_agent(correlation_id.as_deref()).await;
            }

            DomainEvent::InCallApprovalRequested {
                request_id,
                tool_name,
                action_summary,
                correlation_id,
            } => {
                tracing::info!(
                    request_id = %request_id,
                    tool = %tool_name,
                    correlation_id = ?correlation_id,
                    "{LOG_PREFIX} in-call approval parked — speaking prompt into call"
                );
                let action_summary = action_summary.clone();
                let correlation_id = correlation_id.clone();
                tokio::spawn(async move {
                    super::in_call::speak_approval_prompt(
                        &action_summary,
                        correlation_id.as_deref(),
                    )
                    .await;
                });
            }

            DomainEvent::BackendMeetInCallRequest {
                correlation_id,
                speaker,
                command_text,
                recent_transcript,
                timestamp_ms,
                mascot_slot,
            } => {
                tracing::info!(
                    correlation_id = ?correlation_id,
                    speaker = %speaker,
                    cmd_len = command_text.len(),
                    mascot_slot = ?mascot_slot,
                    "{LOG_PREFIX} in-call request received"
                );
                // The orchestrator turn can run for tens of seconds (tools,
                // integrations) — spawn so the event bus isn't blocked.
                let correlation_id = correlation_id.clone();
                let speaker = speaker.clone();
                let command_text = command_text.clone();
                let recent_transcript = recent_transcript.clone();
                let timestamp_ms = *timestamp_ms;
                let mascot_slot = *mascot_slot;
                tokio::spawn(async move {
                    super::in_call::handle_in_call_request(
                        correlation_id,
                        speaker,
                        command_text,
                        recent_transcript,
                        timestamp_ms,
                        mascot_slot,
                    )
                    .await;
                });
            }

            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::openhuman::config::AutoSummarizePolicy;

    #[test]
    fn subscriber_name_is_correct() {
        let subscriber = MeetingEventSubscriber;
        assert_eq!(subscriber.name(), "agent_meetings::events");
    }

    #[test]
    fn subscriber_domains_filter_to_agent_meetings() {
        let subscriber = MeetingEventSubscriber;
        assert_eq!(subscriber.domains(), Some(&["agent_meetings"][..]));
    }

    struct EnvGuard {
        previous: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn set_workspace(path: &std::path::Path) -> Self {
            let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
            std::env::set_var("OPENHUMAN_WORKSPACE", path);
            Self { previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => std::env::set_var("OPENHUMAN_WORKSPACE", value),
                None => std::env::remove_var("OPENHUMAN_WORKSPACE"),
            }
        }
    }

    async fn save_summary_policy(policy: AutoSummarizePolicy) {
        let mut config = crate::openhuman::config::Config::load_or_init()
            .await
            .expect("config loads in temp workspace");
        config.meet.auto_summarize_policy = policy;
        config.meet.ingest_backend_transcripts = false;
        config.save().await.expect("config saves policy");
    }

    async fn send_transcript_for_policy(policy: AutoSummarizePolicy, correlation_id: &str) {
        save_summary_policy(policy).await;
        let event = DomainEvent::BackendMeetTranscript {
            turns: vec![crate::core::event_bus::BackendMeetTurn {
                role: "user".to_string(),
                content: "[00:01] [Alice] ship it".to_string(),
            }],
            duration_ms: 60_000,
            correlation_id: Some(correlation_id.to_string()),
        };
        MeetingEventSubscriber.handle(&event).await;
    }

    async fn has_summary_prompt_marker(meeting_id: &str) -> bool {
        use crate::openhuman::memory::rpc_models::{ConversationMessagesRequest, EmptyRequest};

        let threads = crate::openhuman::threads::ops::threads_list(EmptyRequest {})
            .await
            .expect("list threads")
            .value
            .data
            .expect("threads data")
            .threads;
        for thread in threads {
            let messages =
                crate::openhuman::threads::ops::messages_list(ConversationMessagesRequest {
                    thread_id: thread.id,
                })
                .await
                .expect("list messages")
                .value
                .data
                .expect("messages data")
                .messages;
            if messages.iter().any(|message| {
                message
                    .extra_metadata
                    .get("kind")
                    .and_then(serde_json::Value::as_str)
                    == Some("meeting_summary_prompt")
                    && message
                        .extra_metadata
                        .get("meeting_id")
                        .and_then(serde_json::Value::as_str)
                        == Some(meeting_id)
            }) {
                return true;
            }
        }
        false
    }

    #[tokio::test]
    async fn transcript_policy_respects_never_and_ask_without_summary() {
        let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::TempDir::new().unwrap();
        let _env = EnvGuard::set_workspace(tmp.path());

        send_transcript_for_policy(AutoSummarizePolicy::Never, "policy-never").await;
        let never_detail = crate::openhuman::meet_agent::store::read_detail("policy-never")
            .await
            .expect("read never detail")
            .expect("never detail exists");
        assert!(never_detail.summary.is_none());

        send_transcript_for_policy(AutoSummarizePolicy::Ask, "policy-ask").await;
        let ask_detail = crate::openhuman::meet_agent::store::read_detail("policy-ask")
            .await
            .expect("read ask detail")
            .expect("ask detail exists");
        assert!(ask_detail.summary.is_none());
        assert!(
            has_summary_prompt_marker("policy-ask").await,
            "Ask policy should append a summary prompt marker"
        );
    }
}
