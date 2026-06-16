//! Tests for boot-time concurrent MCP server spawn.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::spawn_servers_concurrently;
use super::BOOT_SPAWN_CONCURRENCY;
use crate::openhuman::mcp_registry::types::{CommandKind, InstalledServer, Transport};

fn sample_server(id: &str, enabled: bool) -> InstalledServer {
    InstalledServer {
        server_id: id.to_string(),
        qualified_name: format!("@test/{id}"),
        display_name: "Test Server".to_string(),
        description: None,
        icon_url: None,
        command_kind: CommandKind::Node,
        command: "npx".to_string(),
        args: vec!["-y".to_string()],
        env_keys: Vec::new(),
        config: None,
        installed_at: 1_700_000_000_000,
        last_connected_at: None,
        transport: Transport::Stdio,
        enabled,
    }
}

/// Tracks how many `connect_fn` invocations overlap so the test can assert
/// real concurrency (peak in-flight > 1) bounded by `BOOT_SPAWN_CONCURRENCY`.
#[derive(Default)]
struct ConcurrencyProbe {
    in_flight: AtomicUsize,
    peak: AtomicUsize,
    calls: AtomicUsize,
}

impl ConcurrencyProbe {
    async fn run(&self) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        // Hold the slot long enough that siblings pile up if run concurrently.
        tokio::time::sleep(Duration::from_millis(40)).await;
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn spawns_enabled_servers_concurrently() {
    let probe = Arc::new(ConcurrencyProbe::default());
    let servers: Vec<InstalledServer> = (0..4)
        .map(|i| sample_server(&format!("srv-{i}"), true))
        .collect();

    let probe_for_fn = probe.clone();
    spawn_servers_concurrently(servers, move |_server| {
        let probe = probe_for_fn.clone();
        async move {
            probe.run().await;
            Ok(3usize)
        }
    })
    .await;

    assert_eq!(
        probe.calls.load(Ordering::SeqCst),
        4,
        "all enabled servers connected"
    );
    let peak = probe.peak.load(Ordering::SeqCst);
    assert!(
        peak >= 2,
        "expected overlapping connects, peak in-flight was {peak}"
    );
    assert!(
        peak <= BOOT_SPAWN_CONCURRENCY,
        "peak in-flight {peak} must not exceed BOOT_SPAWN_CONCURRENCY {BOOT_SPAWN_CONCURRENCY}"
    );
}

#[tokio::test]
async fn skips_disabled_servers() {
    let probe = Arc::new(ConcurrencyProbe::default());
    let servers = vec![
        sample_server("on-1", true),
        sample_server("off-1", false),
        sample_server("on-2", true),
        sample_server("off-2", false),
    ];

    let probe_for_fn = probe.clone();
    spawn_servers_concurrently(servers, move |_server| {
        let probe = probe_for_fn.clone();
        async move {
            probe.run().await;
            Ok(0usize)
        }
    })
    .await;

    assert_eq!(
        probe.calls.load(Ordering::SeqCst),
        2,
        "only the two enabled servers should be connected"
    );
}

/// An error from one connect must not abort the others (boot is best-effort).
#[tokio::test]
async fn one_failure_does_not_abort_the_rest() {
    let probe = Arc::new(ConcurrencyProbe::default());
    let servers: Vec<InstalledServer> = (0..3)
        .map(|i| sample_server(&format!("srv-{i}"), true))
        .collect();

    let probe_for_fn = probe.clone();
    spawn_servers_concurrently(servers, move |server| {
        let probe = probe_for_fn.clone();
        async move {
            probe.run().await;
            if server.server_id == "srv-1" {
                anyhow::bail!("boom");
            }
            Ok(1usize)
        }
    })
    .await;

    assert_eq!(
        probe.calls.load(Ordering::SeqCst),
        3,
        "every server is attempted even when one fails"
    );
}
