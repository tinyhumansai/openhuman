//! Background supervisor that keeps installed MCP servers connected (#3312).
//!
//! Problem: MCP transports drop silently over a long-running deployment — a
//! stdio subprocess exits, or an HTTP-remote session expires — and nothing
//! re-establishes them. Connections are brought up only once, at boot
//! ([`super::boot::spawn_installed_servers`]). After a few hours a headless
//! deployment ends up with 0 MCP tools and no way back short of a restart.
//!
//! This supervisor mirrors the cron scheduler's tick loop: every
//! [`TICK_INTERVAL`] it walks every enabled installed server and, for each:
//! - if it is in the registry, actively probes the transport
//!   ([`super::connections::probe_alive`]) — a silent drop only surfaces under
//!   an actual round-trip, so map membership alone is not trusted;
//! - if the probe fails, disconnects the dead transport and reconnects;
//! - if it is not connected, reconnects — subject to per-server exponential
//!   backoff so a genuinely-down or misconfigured server isn't hammered.
//!
//! Scope: this PR is connectivity only. The supervisor deliberately does **not**
//! publish health events to the global health bus — doing so would flip the
//! whole container to `unhealthy`/503 whenever a single MCP server is down,
//! which is exactly the all-or-nothing coupling the granular-health work
//! addresses separately. Reconnect outcomes are logged.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::openhuman::config::Config;

use super::{connections, store};

/// How often the supervisor walks the installed-server list.
const TICK_INTERVAL: Duration = Duration::from_secs(60);

/// Per-server liveness-probe timeout. A `tools/list` round-trip should be fast;
/// a server that can't answer within this window is treated as dropped.
const PROBE_TIMEOUT: Duration = Duration::from_secs(8);

/// Base unit for exponential reconnect backoff.
const BACKOFF_BASE: Duration = Duration::from_secs(5);

/// Cap on reconnect backoff so a long-down server is still retried every few
/// minutes (its operator may fix it without touching OpenHuman).
const BACKOFF_MAX: Duration = Duration::from_secs(300);

/// Exponential backoff delay after `failures` consecutive failed reconnects:
/// `BASE * 2^(failures-1)`, capped at [`BACKOFF_MAX`]. `failures == 0` yields
/// [`BACKOFF_BASE`] (treated as "no failures yet → try immediately-ish").
fn backoff_delay(failures: u32) -> Duration {
    if failures == 0 {
        return BACKOFF_BASE;
    }
    // Saturating shift so a large failure count can't overflow; cap below.
    let shifted = BACKOFF_BASE
        .as_secs()
        .saturating_mul(1u64.checked_shl(failures - 1).unwrap_or(u64::MAX));
    Duration::from_secs(shifted.min(BACKOFF_MAX.as_secs()))
}

/// Per-server reconnect backoff state.
#[derive(Default)]
struct BackoffState {
    failures: u32,
    next_attempt_at: Option<Instant>,
}

impl BackoffState {
    /// Whether a reconnect may be attempted at `now`. A fresh state (no prior
    /// failure) is always ready.
    fn ready(&self, now: Instant) -> bool {
        self.next_attempt_at.is_none_or(|t| now >= t)
    }

    /// Record a failed reconnect and schedule the next eligible attempt.
    fn record_failure(&mut self, now: Instant) {
        self.failures = self.failures.saturating_add(1);
        self.next_attempt_at = Some(now + backoff_delay(self.failures));
    }
}

/// Run the supervisor loop forever. Spawned once at core startup, after the
/// boot connect pass. The first tick is delayed by [`TICK_INTERVAL`] so it does
/// not race the boot spawn.
pub async fn run(config: Config) {
    let start = Instant::now() + TICK_INTERVAL;
    let mut interval = tokio::time::interval_at(start.into(), TICK_INTERVAL);
    let mut backoff: HashMap<String, BackoffState> = HashMap::new();

    tracing::info!(
        "[mcp-supervisor] started: tick={}s probe_timeout={}s",
        TICK_INTERVAL.as_secs(),
        PROBE_TIMEOUT.as_secs()
    );

    loop {
        interval.tick().await;
        tick_once(&config, &mut backoff, Instant::now()).await;
    }
}

/// Run exactly one supervision cycle with a fresh backoff map. Exposed for
/// integration tests (the internal [`tick_once`] can't be `pub` because its
/// `BackoffState` parameter is a private type). Not used by production code —
/// the live loop calls [`tick_once`] directly with persistent backoff state.
#[doc(hidden)]
pub async fn run_single_tick_for_test(config: &Config) {
    let mut backoff: HashMap<String, BackoffState> = HashMap::new();
    tick_once(config, &mut backoff, Instant::now()).await;
}

/// One supervision cycle, extracted so the loop body is driven without owning a
/// `tokio::time::interval`. `now` is injected for deterministic backoff timing.
async fn tick_once(config: &Config, backoff: &mut HashMap<String, BackoffState>, now: Instant) {
    let servers = match store::list_servers(config) {
        Ok(s) => s,
        Err(err) => {
            tracing::warn!("[mcp-supervisor] tick: list_servers failed: {err}");
            return;
        }
    };

    for server in servers {
        let id = server.server_id.clone();

        if !server.enabled {
            // Drop any stale backoff for a server that's been disabled; the
            // disable flow owns tearing down its live connection.
            backoff.remove(&id);
            continue;
        }

        if connections::is_connected(&id).await {
            if connections::probe_alive(&id, PROBE_TIMEOUT).await {
                // Healthy — clear any lingering backoff and move on.
                backoff.remove(&id);
                continue;
            }
            tracing::warn!(
                "[mcp-supervisor] server_id={id} qualified={} transport dropped; reconnecting",
                server.qualified_name
            );
            connections::disconnect(&id).await;
        }

        // Not connected (never connected, or just-disconnected dead transport).
        // Gate the reconnect attempt on the per-server backoff schedule.
        let ready = {
            let st = backoff.entry(id.clone()).or_default();
            st.ready(now)
        };
        if !ready {
            continue;
        }

        match connections::connect(config, &server).await {
            Ok(tools) => {
                backoff.remove(&id);
                tracing::info!(
                    "[mcp-supervisor] reconnected server_id={id} qualified={} tools={}",
                    server.qualified_name,
                    tools.len()
                );
            }
            Err(err) => {
                let st = backoff.entry(id.clone()).or_default();
                st.record_failure(now);
                tracing::warn!(
                    "[mcp-supervisor] reconnect failed server_id={id} qualified={} \
                     failures={} next_retry_in={}s err={err}",
                    server.qualified_name,
                    st.failures,
                    backoff_delay(st.failures).as_secs()
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_delay_grows_exponentially_then_caps() {
        assert_eq!(backoff_delay(0), BACKOFF_BASE);
        assert_eq!(backoff_delay(1), Duration::from_secs(5));
        assert_eq!(backoff_delay(2), Duration::from_secs(10));
        assert_eq!(backoff_delay(3), Duration::from_secs(20));
        assert_eq!(backoff_delay(4), Duration::from_secs(40));
        // Caps at BACKOFF_MAX and never overflows for absurd failure counts.
        assert_eq!(backoff_delay(20), BACKOFF_MAX);
        assert_eq!(backoff_delay(u32::MAX), BACKOFF_MAX);
    }

    #[test]
    fn fresh_state_is_ready_immediately() {
        let st = BackoffState::default();
        assert!(st.ready(Instant::now()));
    }

    #[test]
    fn record_failure_defers_next_attempt_until_backoff_elapses() {
        let base = Instant::now();
        let mut st = BackoffState::default();
        st.record_failure(base);
        assert_eq!(st.failures, 1);
        // Not ready immediately after a failure...
        assert!(!st.ready(base));
        assert!(!st.ready(base + Duration::from_secs(4)));
        // ...ready once the 5s base backoff has elapsed.
        assert!(st.ready(base + Duration::from_secs(5)));
    }

    #[test]
    fn consecutive_failures_lengthen_the_backoff_window() {
        let base = Instant::now();
        let mut st = BackoffState::default();
        st.record_failure(base); // failures=1 → 5s
        st.record_failure(base); // failures=2 → 10s from `base`
        assert_eq!(st.failures, 2);
        assert!(!st.ready(base + Duration::from_secs(9)));
        assert!(st.ready(base + Duration::from_secs(10)));
    }
}
