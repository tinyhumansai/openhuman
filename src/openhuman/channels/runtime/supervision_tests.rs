use super::super::super::{traits, Channel};
use super::{jitter_millis, spawn_supervised_listener, MAX_JITTER_MS};
use std::sync::{Arc, Mutex};
use tokio::time::Instant;

#[test]
fn supervision_discord_gateway_reqwest_failure_classifies_as_expected() {
    let raw = "error sending request for url (https://discord.com/api/v10/gateway/bot)";
    let wrapped = format!("Channel discord error: {raw}; restarting");
    let kind = crate::core::observability::expected_error_kind(&wrapped);
    assert_eq!(
        kind,
        Some(crate::core::observability::ExpectedErrorKind::ChannelSupervisorRestart),
        "supervision wrapper must classify as ChannelSupervisorRestart \
         (precedence over NetworkUnreachable) so Sentry stays quiet for \
         TAURI-RUST-15/-BB (got {kind:?} for message {wrapped:?})"
    );
}

#[test]
fn jitter_millis_is_bounded_by_backoff_and_max() {
    // Sub-second backoff windows scale with the backoff; large ones clamp.
    for _ in 0..256 {
        assert!(
            jitter_millis(1) < 1_000,
            "1s backoff jitters within [0,1000)"
        );
        assert!(
            jitter_millis(60) < MAX_JITTER_MS,
            "large backoff jitter must clamp to MAX_JITTER_MS"
        );
    }
    // Degenerate window: no room to sample, must not panic on `0..0`.
    assert_eq!(jitter_millis(0), 0, "zero backoff yields zero jitter");
}

/// Fake channel whose `listen()` returns `Ok(())` immediately — mirroring
/// Discord's op7/op9/Close paths, which all return `Ok(())` (see the
/// tinychannels `DiscordChannel::listen` gateway loop). It records the
/// virtual-clock instant of every `listen` entry and, once it has been
/// restarted `stop_after` times, drops the receiver it holds so the
/// supervisor's `tx.is_closed()` guard breaks the loop deterministically.
struct ReconnectOkChannel {
    entries: Arc<Mutex<Vec<Instant>>>,
    stop_after: usize,
    // Held solely to keep `tx.is_closed()` false until we choose to stop;
    // dropped in-place on the final `listen` to break the supervisor loop.
    rx: Mutex<Option<tokio::sync::mpsc::Receiver<traits::ChannelMessage>>>,
}

#[async_trait::async_trait]
impl Channel for ReconnectOkChannel {
    fn name(&self) -> &str {
        "reconnect-ok"
    }

    async fn send(&self, _message: &super::super::super::SendMessage) -> anyhow::Result<()> {
        Ok(())
    }

    async fn listen(
        &self,
        _tx: tokio::sync::mpsc::Sender<traits::ChannelMessage>,
    ) -> anyhow::Result<()> {
        let count = {
            let mut entries = self.entries.lock().expect("entries lock");
            entries.push(Instant::now());
            entries.len()
        };
        if count >= self.stop_after {
            // Drop the receiver so the next `tx.is_closed()` check breaks
            // the supervisor loop instead of spinning forever.
            self.rx.lock().expect("rx lock").take();
        }
        Ok(())
    }
}

/// Regression for #5350: a reconnect-style `Ok(())` exit (Discord op7/op9/
/// Close) must ESCALATE the restart backoff, not reset it to the initial
/// value every iteration. Before the fix the `Ok(())` arm reset
/// `backoff = initial_backoff_secs`, so a reconnect storm looped at a flat
/// ~2s forever; the gaps between successive `listen` entries would all be
/// ~1s here. With the fix the base backoff doubles (1 → 2 → 4 …) exactly
/// like the `Err` path, so the whole-second gaps are 1, 2, 4.
///
/// Runs under a paused clock, so the supervisor's `sleep(backoff)` calls
/// auto-advance virtual time and the test completes without real waits.
#[tokio::test(start_paused = true)]
async fn reconnect_ok_exit_escalates_backoff_instead_of_resetting() {
    // A live bus is needed for the ChannelConnected/HealthRestarted publishes
    // the supervisor emits each iteration (matches tests/health.rs setup).
    crate::core::bus::init().await.expect("bus init");

    let entries = Arc::new(Mutex::new(Vec::new()));
    // Stop after the 4th restart so we observe three inter-restart gaps
    // (backoff steps 1, 2, 4) before the loop breaks.
    let (tx, rx) = tokio::sync::mpsc::channel::<traits::ChannelMessage>(1);
    let channel: Arc<dyn Channel> = Arc::new(ReconnectOkChannel {
        entries: Arc::clone(&entries),
        stop_after: 4,
        rx: Mutex::new(Some(rx)),
    });

    // initial=1, max=8 so the 1 → 2 → 4 escalation is fully visible before
    // the cap would bite.
    let handle = spawn_supervised_listener(channel, tx, 1, 8);
    handle.await.expect("supervised listener task");

    let entries = entries.lock().expect("entries lock");
    assert_eq!(
        entries.len(),
        4,
        "listener should have been restarted exactly stop_after times, got {}",
        entries.len()
    );

    // Whole-second gaps between successive restarts. Jitter is strictly
    // < 1s (bounded by MAX_JITTER_MS), so the seconds floor isolates the
    // base backoff exactly: escalating 1, 2, 4 — never the flat 1, 1, 1
    // the pre-fix reset produced.
    let gap_secs: Vec<u64> = entries
        .windows(2)
        .map(|w| w[1].duration_since(w[0]).as_secs())
        .collect();
    assert_eq!(
        gap_secs,
        vec![1, 2, 4],
        "reconnect-style Ok(()) exits must escalate backoff (1,2,4), not reset it \
         to the flat initial delay (which would show 1,1,1); got {gap_secs:?}"
    );
}
