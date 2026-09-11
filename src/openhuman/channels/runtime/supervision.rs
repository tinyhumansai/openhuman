//! Supervisor helpers for channel listeners.

use super::super::traits;
use super::super::Channel;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use std::sync::Arc;
use std::time::Duration;

pub(crate) use tinychannels::runtime::compute_max_in_flight_messages;

/// Upper bound on reconnect jitter, regardless of how large `backoff` has grown.
/// One second of spread is plenty to de-correlate lockstep reconnects while
/// staying inside Discord's 1-5s post-op9 window on the low backoff steps.
const MAX_JITTER_MS: u64 = 1_000;

/// Randomized delay to add on top of the base backoff before a reconnect, in
/// millis. Full-jitter over `[0, backoff_secs * 1000)` ms, clamped to
/// [`MAX_JITTER_MS`] so a large backoff doesn't produce an unboundedly long
/// extra wait. Returns `0` when there is no window to sample (defensive — the
/// supervisor's `backoff` is always `>= 1`, so this only guards misuse).
///
/// Uses the crate-wide `rand::rng()` CSPRNG idiom (rand 0.10); the window math
/// is deterministic so the bound is unit-testable independent of the sample.
fn jitter_millis(backoff_secs: u64) -> u64 {
    use rand::RngExt as _;
    let window_ms = backoff_secs.saturating_mul(1_000).min(MAX_JITTER_MS);
    if window_ms == 0 {
        return 0;
    }
    rand::rng().random_range(0..window_ms)
}

pub(crate) fn spawn_supervised_listener(
    ch: Arc<dyn Channel>,
    tx: tokio::sync::mpsc::Sender<traits::ChannelMessage>,
    initial_backoff_secs: u64,
    max_backoff_secs: u64,
) -> tokio::task::JoinHandle<()> {
    // Health events need a live bus, but standing one up is async now and this
    // helper is not. Registering the subscriber is still sync and still safe
    // before init; the bus itself is initialised once at startup, and a publish
    // that lands before that is a documented no-op rather than a panic.
    crate::openhuman::platform::health::bus::register_health_subscriber();

    tokio::spawn(async move {
        let component = format!("channel:{}", ch.name());
        let mut backoff = initial_backoff_secs.max(1);
        let max_backoff = max_backoff_secs.max(backoff);

        tracing::info!(
            channel = ch.name(),
            initial_backoff_secs,
            max_backoff_secs,
            "[channels] supervised listener started"
        );

        loop {
            BUS.publish(DomainEvent::ChannelConnected {
                channel: ch.name().to_string(),
            });
            tracing::debug!(
                channel = ch.name(),
                "[channels] listener entering recv loop"
            );
            let result = ch.listen(tx.clone()).await;

            if tx.is_closed() {
                break;
            }

            match result {
                Ok(()) => {
                    // A supervisor asked to stop closes `tx`, which is caught by
                    // the `tx.is_closed()` break above — so any `Ok(())` that
                    // reaches here is a reconnect-style exit, not an intentional
                    // clean shutdown. Discord's gateway returns `Ok(())` on op7
                    // (reconnect), op9 (invalid session), and Close frames; a
                    // reconnect/invalid-session storm therefore lands here
                    // repeatedly. Resetting backoff on this path made it spin at
                    // the flat initial delay forever, hammering Discord IDENTIFY
                    // and pinning the CPU (#5350). Fall through to the shared
                    // escalation below instead — the only difference from `Err`
                    // is that this exit is expected and not reported.
                    tracing::warn!("Channel {} exited unexpectedly; restarting", ch.name());
                    BUS.publish(DomainEvent::ChannelDisconnected {
                        channel: ch.name().to_string(),
                        reason: "exited unexpectedly".to_string(),
                    });
                }
                Err(e) => {
                    let message = format!("Channel {} error: {e:#}; restarting", ch.name());
                    crate::core::observability::report_error_or_expected(
                        message.as_str(),
                        "channels",
                        "supervised_listener",
                        &[("channel", ch.name())],
                    );
                    BUS.publish(DomainEvent::ChannelDisconnected {
                        channel: ch.name().to_string(),
                        reason: e.to_string(),
                    });
                }
            }

            BUS.publish(DomainEvent::HealthRestarted {
                component: component.clone(),
            });
            // Full-jitter on top of the base backoff so many channels/instances
            // don't reconnect in lockstep (Discord asks for a randomized 1-5s
            // wait after op9 before re-IDENTIFY, and lockstep reconnects across
            // channels amplify IDENTIFY rate-limiting). Jitter spans [0, backoff)
            // seconds, expressed in millis, capped so it never dwarfs the base.
            let jitter_ms = jitter_millis(backoff);
            tokio::time::sleep(Duration::from_secs(backoff) + Duration::from_millis(jitter_ms))
                .await;
            // Double backoff AFTER sleeping so the first restart uses initial_backoff
            backoff = backoff.saturating_mul(2).min(max_backoff);
        }
    })
}

#[cfg(test)]
#[path = "supervision_tests.rs"]
mod tests;
