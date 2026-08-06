//! Background scheduler for the stability detector rebuild cycle.
//!
//! Provides two scheduling mechanisms:
//!
//! 1. **Periodic timer** — `spawn_rebuild_loop` runs a rebuild every `interval`
//!    (default 30 minutes).
//!
//! 2. **Event-driven trigger** — `spawn_event_driven_trigger` subscribes to
//!    [`DomainEvent::DocumentCanonicalized`] and [`DomainEvent::TreeSummarizerPropagated`]
//!    events and schedules a debounced out-of-cycle rebuild 60 seconds after receipt.
//!
//! Both mechanisms respect a shutdown signal from a
//! `tokio::sync::watch::Receiver<bool>` channel.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use tokio::sync::watch;

use crate::core::event_bus::{subscribe_global, DomainEvent, EventHandler, SubscriptionHandle};
// Arc imported for detector sharing across async tasks
use crate::openhuman::agent::learning::stability_detector::StabilityDetector;

// ── Default intervals ─────────────────────────────────────────────────────────

/// Default periodic rebuild interval.
pub const DEFAULT_REBUILD_INTERVAL: Duration = Duration::from_secs(30 * 60);

/// Debounce delay for event-driven rebuilds — we wait this long after the last
/// triggering event before running the rebuild.
const EVENT_REBUILD_DELAY: Duration = Duration::from_secs(60);

// ── Periodic rebuild loop ─────────────────────────────────────────────────────

/// Spawn a background task that runs a stability detector rebuild every `interval`.
///
/// The task shuts down when `shutdown_rx` signals `true` or when the tokio
/// runtime exits.
///
/// # Arguments
///
/// * `detector` — the rebuild engine to call.
/// * `interval` — how often to rebuild (default: [`DEFAULT_REBUILD_INTERVAL`]).
/// * `shutdown_rx` — watch channel; the task exits when the value becomes `true`.
pub fn spawn_rebuild_loop(
    detector: Arc<StabilityDetector>,
    interval: Duration,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    tracing::info!(
        "[learning::scheduler] starting periodic rebuild loop (interval={}s)",
        interval.as_secs()
    );

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        // Skip the first immediate tick so we don't rebuild at startup before
        // any producers have had a chance to emit candidates.
        ticker.tick().await;

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    run_rebuild_logged(&detector, "periodic").await;
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        tracing::info!("[learning::scheduler] shutdown signal received, stopping rebuild loop");
                        break;
                    }
                }
            }
        }

        tracing::info!("[learning::scheduler] rebuild loop stopped");
    });
}

// ── Event-driven trigger ──────────────────────────────────────────────────────

/// Event handler that schedules a debounced rebuild on relevant domain events.
///
/// Listens for:
/// - [`DomainEvent::DocumentCanonicalized`] with `source_kind == "email"` or `"document"`
/// - [`DomainEvent::TreeSummarizerPropagated`] (tree summariser flush signal)
struct RebuildTriggerHandler {
    detector: Arc<StabilityDetector>,
}

#[async_trait]
impl EventHandler for RebuildTriggerHandler {
    fn name(&self) -> &str {
        "learning::rebuild_trigger"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["memory", "tree_summarizer"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let should_trigger = match event {
            DomainEvent::DocumentCanonicalized { source_kind, .. } => {
                matches!(source_kind.as_str(), "email" | "document")
            }
            DomainEvent::TreeSummarizerPropagated { .. } => true,
            _ => false,
        };

        if !should_trigger {
            return;
        }

        tracing::debug!(
            "[learning::scheduler] event-driven rebuild triggered, scheduling in {}s",
            EVENT_REBUILD_DELAY.as_secs()
        );

        let detector = Arc::clone(&self.detector);
        let delay = EVENT_REBUILD_DELAY;

        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            run_rebuild_logged(&detector, "event-driven").await;
        });
    }
}

/// Register the event-driven trigger subscriber.
///
/// The returned `SubscriptionHandle` must be kept alive for the subscription to
/// remain active. Callers should store it in a static `OnceLock` (same pattern
/// as the `EmailSignatureSubscriber`).
pub fn register_event_trigger(detector: Arc<StabilityDetector>) -> Option<SubscriptionHandle> {
    subscribe_global(Arc::new(RebuildTriggerHandler { detector }))
}

// ── Shared rebuild runner ─────────────────────────────────────────────────────

async fn run_rebuild_logged(detector: &StabilityDetector, source: &str) {
    let now = now_secs();
    match detector.rebuild(now) {
        Ok(outcome) => {
            tracing::info!(
                "[learning::scheduler] {source} rebuild complete: \
                 added={} evicted={} kept={} total={}",
                outcome.added,
                outcome.evicted,
                outcome.kept,
                outcome.total_size,
            );
        }
        Err(e) => {
            tracing::warn!("[learning::scheduler] {source} rebuild failed (non-fatal): {e}");
        }
    }
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
