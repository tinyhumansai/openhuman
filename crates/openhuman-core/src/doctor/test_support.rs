//! Reusable fake checks and sources for doctor framework tests.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use super::contract::{DoctorCheck, DoctorCtx, DoctorTier, Finding};
use super::registry::DoctorCheckSource;

/// A configurable check used by framework and fan-out tests.
pub(crate) struct FakeCheck {
    id: String,
    findings: Vec<Finding>,
    delay: Duration,
    timeout: Option<Duration>,
    tier: DoctorTier,
    calls: Option<Arc<AtomicUsize>>,
}

impl FakeCheck {
    pub(crate) fn pass(id: impl Into<String>) -> Self {
        let id = id.into();
        Self {
            findings: vec![Finding::pass(&id, "The fake check passed.")],
            id,
            delay: Duration::ZERO,
            timeout: None,
            tier: DoctorTier::Connect,
            calls: None,
        }
    }

    pub(crate) fn sleep_for(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    pub(crate) fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub(crate) fn with_tier(mut self, tier: DoctorTier) -> Self {
        self.tier = tier;
        self
    }

    pub(crate) fn with_call_counter(mut self, calls: Arc<AtomicUsize>) -> Self {
        self.calls = Some(calls);
        self
    }
}

#[async_trait]
impl DoctorCheck for FakeCheck {
    fn id(&self) -> &str {
        &self.id
    }

    fn category(&self) -> &str {
        "test"
    }

    fn tier(&self) -> DoctorTier {
        self.tier
    }

    fn timeout(&self) -> Duration {
        self.timeout.unwrap_or_else(|| self.tier.timeout())
    }

    async fn detect(&self, _ctx: &DoctorCtx) -> Vec<Finding> {
        if let Some(calls) = &self.calls {
            calls.fetch_add(1, Ordering::SeqCst);
        }
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        self.findings.clone()
    }
}

/// A source that contributes fake checks to a single registry scope.
pub(crate) struct FakeCheckSource {
    id: String,
    checks: Vec<Arc<dyn DoctorCheck>>,
}

impl FakeCheckSource {
    pub(crate) fn new(id: impl Into<String>, checks: Vec<FakeCheck>) -> Self {
        Self {
            id: id.into(),
            checks: checks
                .into_iter()
                .map(|check| Arc::new(check) as Arc<dyn DoctorCheck>)
                .collect(),
        }
    }
}

impl DoctorCheckSource for FakeCheckSource {
    fn id(&self) -> &str {
        &self.id
    }

    fn checks(&self) -> Vec<Arc<dyn DoctorCheck>> {
        self.checks.clone()
    }
}
