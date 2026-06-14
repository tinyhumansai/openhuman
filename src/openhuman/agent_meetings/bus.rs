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
