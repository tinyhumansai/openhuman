//! Manual in-process bus latency recorder. Run explicitly with
//! `cargo test -p openhuman-cli --test security_bus_latency_baseline -- --ignored --nocapture`.
//! This reports measurements only; it has no timing threshold and makes no
//! backend or network calls.

use std::{sync::Arc, time::Instant};

use openhuman_core::core::{bus_testing::isolated_bus, events::DomainEvent};
use tinybus::EventHandler;
use tokio::sync::mpsc;

struct Capture(mpsc::UnboundedSender<()>);

#[async_trait::async_trait]
impl EventHandler<DomainEvent> for Capture {
    fn name(&self) -> &str {
        "security::latency-baseline"
    }

    async fn handle(&self, _event: &DomainEvent) {
        let _ = self.0.send(());
    }
}

#[tokio::test]
#[ignore = "manual baseline capture; run with --ignored --nocapture"]
async fn records_local_publish_to_handler_latency_percentiles() {
    const SAMPLES: usize = 1_000;
    let bus = isolated_bus().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let _subscription = bus.subscribe(Arc::new(Capture(tx)));

    let mut samples = Vec::with_capacity(SAMPLES);
    for index in 0..SAMPLES {
        let started = Instant::now();
        bus.publish_awaited(DomainEvent::SystemStartup {
            component: format!("security-baseline-{index}"),
        })
        .await
        .expect("in-process event should publish");
        rx.recv().await.expect("local handler should receive event");
        samples.push(started.elapsed().as_nanos());
    }
    samples.sort_unstable();
    let percentile = |p: usize| samples[(SAMPLES * p / 100).saturating_sub(1)];
    println!(
        "samples={SAMPLES} p50_ns={} p95_ns={} p99_ns={} max_ns={}",
        percentile(50),
        percentile(95),
        percentile(99),
        samples[SAMPLES - 1]
    );
}
