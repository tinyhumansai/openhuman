//! Event-bus subscriber that reacts to backend meeting events.
//!
//! - `BackendMeetTranscript` → creates a dedicated "Meetings"-labelled
//!   conversation thread and appends the transcript.
//! - `BackendMeetJoined` / `BackendMeetLeft` → logged for audit trail;
//!   session status tracking is handled by the frontend Redux slice.

use std::sync::OnceLock;

use async_trait::async_trait;

use crate::core::event_bus::{DomainEvent, EventHandler, SubscriptionHandle};
use crate::openhuman::notifications::bus::publish_core_notification;
use crate::openhuman::notifications::types::{
    CoreNotificationAction, CoreNotificationCategory, CoreNotificationEvent,
};

use super::ops::{create_meeting_thread_with_transcript, ingest_backend_meeting_transcript};
use super::summary::{summary_action, SummaryAction};

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

                // Load config once so both the summary-policy gate below and the
                // memory-ingest gate further down read a single snapshot. On a
                // config load failure we fall back to the default policy (Ask),
                // which never auto-summarises — safer than the old unconditional
                // behaviour.
                let config = crate::openhuman::config::Config::load_or_init().await.ok();
                let policy = config
                    .as_ref()
                    .map(|c| c.meet.auto_summarize_policy)
                    .unwrap_or_default();
                let action = summary_action(policy);

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
                //    patched in by step 4 once it's ready. This transcript-only
                //    detail is also what keeps the recent-call panel working under
                //    the Never/Ask policies (no summary, transcript intact).
                super::recent_calls::record_backend_call_detail(&request_id, turns, None).await;

                // 3. Generate the post-call summary — but only when the user's
                //    auto_summarize_policy says to (Always). Under Ask/Never we
                //    skip the LLM call entirely; the Ask card below lets the user
                //    opt in afterwards. When generated, this single summary is
                //    shared by the call-detail store (step 4) and the meeting
                //    thread (step 5) so it isn't paid for twice.
                let generated = match action {
                    SummaryAction::Generate => {
                        super::summary::generate_meeting_summary_bounded(
                            turns,
                            correlation_id.as_deref(),
                        )
                        .await
                    }
                    SummaryAction::Ask | SummaryAction::Skip => {
                        tracing::debug!(
                            ?policy,
                            "{LOG_PREFIX} auto-summary skipped per auto_summarize_policy"
                        );
                        None
                    }
                };

                // 4. Upgrade the stored detail with the summary once it's ready.
                //    Skipped when summarisation was gated off or failed/timed out —
                //    the transcript written in step 2 stands on its own.
                if generated.is_some() {
                    super::recent_calls::record_backend_call_detail(
                        &request_id,
                        turns,
                        generated.as_ref(),
                    )
                    .await;
                }

                // 5. Create the meeting thread with transcript, reusing the
                //    summary generated in step 3 (when any). Self-generation is
                //    disabled here because the policy decision was already made
                //    above — under Ask/Never the thread carries the transcript
                //    alone.
                if let Err(e) = create_meeting_thread_with_transcript(
                    turns,
                    *duration_ms,
                    correlation_id.clone(),
                    generated.as_ref(),
                    false,
                )
                .await
                {
                    tracing::warn!("{LOG_PREFIX} meeting thread creation failed: {e}");
                }

                // 6. Under the Ask policy, surface an approval card so the user
                //    can opt into a summary after the call. Confirming routes
                //    back through agent_meetings_notification_action → the
                //    on-demand summary path, keyed by the recorded call id.
                if matches!(action, SummaryAction::Ask) {
                    let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
                    let delivered = publish_core_notification(build_summary_ask_notification(
                        &request_id,
                        now_ms,
                    ));
                    tracing::info!(
                        request_id = %request_id,
                        delivered,
                        "{LOG_PREFIX} posted post-call summary Ask card"
                    );
                }

                // Also ingest into memory tree (existing pipeline).
                let enabled = config
                    .as_ref()
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
            } => {
                tracing::info!(
                    correlation_id = ?correlation_id,
                    speaker = %speaker,
                    cmd_len = command_text.len(),
                    "{LOG_PREFIX} in-call request received"
                );
                // The orchestrator turn can run for tens of seconds (tools,
                // integrations) — spawn so the event bus isn't blocked.
                let correlation_id = correlation_id.clone();
                let speaker = speaker.clone();
                let command_text = command_text.clone();
                let recent_transcript = recent_transcript.clone();
                let timestamp_ms = *timestamp_ms;
                tokio::spawn(async move {
                    super::in_call::handle_in_call_request(
                        correlation_id,
                        speaker,
                        command_text,
                        recent_transcript,
                        timestamp_ms,
                    )
                    .await;
                });
            }

            _ => {}
        }
    }
}

/// Build the "Meeting ended — summarise?" approval card shown when the user's
/// `auto_summarize_policy` is `Ask`. Confirming routes back through
/// `agent_meetings_notification_action` with `action_id = "summarize"` and the
/// recorded call's id (`payload.meetingId`), which drives the on-demand summary
/// path. Pure over its inputs so the card shape is unit-testable.
fn build_summary_ask_notification(request_id: &str, now_ms: u64) -> CoreNotificationEvent {
    let payload = serde_json::json!({ "meetingId": request_id });
    let action = |action_id: &str, label: &str| CoreNotificationAction {
        action_id: action_id.to_string(),
        label: label.to_string(),
        payload: Some(payload.clone()),
    };
    CoreNotificationEvent {
        id: format!("meet-summary-ask:{request_id}"),
        category: CoreNotificationCategory::Meetings,
        title: "Meeting ended".to_string(),
        body: "Want me to summarise this call?".to_string(),
        deep_link: None,
        timestamp_ms: now_ms,
        actions: Some(vec![
            action("summarize", "Summarise"),
            action("dismiss", "No thanks"),
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn summary_ask_notification_carries_summarize_and_dismiss_actions() {
        let n = build_summary_ask_notification("call-7", 1_234);
        assert_eq!(n.id, "meet-summary-ask:call-7");
        assert_eq!(n.category, CoreNotificationCategory::Meetings);
        assert_eq!(n.timestamp_ms, 1_234);

        let actions = n.actions.expect("actions present");
        let ids: Vec<&str> = actions.iter().map(|a| a.action_id.as_str()).collect();
        assert_eq!(ids, vec!["summarize", "dismiss"]);

        // Both buttons carry the recorded call id so the action handler can find
        // the transcript to summarise.
        for a in &actions {
            let meeting_id = a
                .payload
                .as_ref()
                .and_then(|p| p.get("meetingId"))
                .and_then(|v| v.as_str());
            assert_eq!(meeting_id, Some("call-7"));
        }
    }
}
