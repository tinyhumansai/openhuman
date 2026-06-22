//! Event-bus subscriber that reacts to backend meeting events.
//!
//! - `BackendMeetTranscript` → creates a dedicated "Meetings"-labelled
//!   conversation thread and appends the transcript.
//! - `BackendMeetJoined` / `BackendMeetLeft` → logged for audit trail;
//!   session status tracking is handled by the frontend Redux slice.

use std::sync::OnceLock;

use async_trait::async_trait;

use crate::core::event_bus::{DomainEvent, EventHandler, SubscriptionHandle};

use super::ops::{create_meeting_thread_with_transcript, ingest_backend_meeting_transcript};

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

                // Record a recent-calls entry (meet id, duration, owner,
                // participants) so the meeting-bots panel shows call history.
                // Done first (before the heavier thread-creation path) so the
                // record is on disk by the time the panel refetches at call-end.
                // Best-effort: never blocks; logs on failure internally.
                super::recent_calls::record_backend_call(
                    turns,
                    *duration_ms,
                    correlation_id.as_deref(),
                )
                .await;

                // Create the meeting thread with transcript.
                if let Err(e) = create_meeting_thread_with_transcript(
                    turns,
                    *duration_ms,
                    correlation_id.clone(),
                )
                .await
                {
                    tracing::warn!("{LOG_PREFIX} meeting thread creation failed: {e}");
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
}
