use super::*;
use crate::openhuman::providers::traits::ProviderCapabilities;
use crate::openhuman::routing::health::LocalHealthChecker;
use crate::openhuman::routing::policy::RoutingHints;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

// ── Mock provider ──────────────────────────────────────────────────────

struct MockProvider {
    name: &'static str,
    calls: AtomicUsize,
    last_model: parking_lot::Mutex<String>,
    fail: AtomicBool,
    /// Fixed response text (controls quality check outcomes).
    response: parking_lot::Mutex<String>,
}

impl MockProvider {
    fn new(name: &'static str, response: &'static str) -> Arc<Self> {
        Arc::new(Self {
            name,
            calls: AtomicUsize::new(0),
            last_model: parking_lot::Mutex::new(String::new()),
            fail: AtomicBool::new(false),
            response: parking_lot::Mutex::new(response.to_string()),
        })
    }

    fn set_fail(&self, v: bool) {
        self.fail.store(v, Ordering::SeqCst);
    }

    fn set_response(&self, r: &str) {
        *self.response.lock() = r.to_string();
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn last_model(&self) -> String {
        self.last_model.lock().clone()
    }
}

#[async_trait]
impl Provider for Arc<MockProvider> {
    async fn chat_with_system(
        &self,
        _system: Option<&str>,
        _msg: &str,
        model: &str,
        _temp: f64,
    ) -> Result<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.last_model.lock() = model.to_string();
        if self.fail.load(Ordering::SeqCst) {
            anyhow::bail!("{} intentional failure", self.name);
        }
        Ok(self.response.lock().clone())
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: true,
            vision: false,
        }
    }
}

/// Build the routing provider with controllable health and hints.
fn router(
    local: Arc<MockProvider>,
    remote: Arc<MockProvider>,
    health: Arc<LocalHealthChecker>,
    hints: RoutingHints,
) -> IntelligentRoutingProvider {
    IntelligentRoutingProvider::with_hints(
        Box::new(remote),
        Box::new(local),
        "gemma3:4b-it-qat".to_string(),
        "default-remote-model".to_string(),
        true,
        health,
        hints,
    )
}

// ── A. Local success path ──────────────────────────────────────────────

#[tokio::test]
async fn local_used_when_healthy_and_lightweight() {
    // Local is healthy → lightweight task must go to local.
    let local = MockProvider::new("local", "Great reaction!");
    let remote = MockProvider::new("remote", "remote-resp");
    let health = LocalHealthChecker::seeded(true);

    let r = router(
        Arc::clone(&local),
        Arc::clone(&remote),
        health,
        RoutingHints::default(),
    );
    let result = r
        .chat_with_system(None, "React to this", "hint:reaction", 0.7)
        .await
        .unwrap();

    assert_eq!(result, "Great reaction!");
    assert_eq!(local.calls(), 1, "local must have been called");
    assert_eq!(remote.calls(), 0, "remote must NOT have been called");
    assert_eq!(local.last_model(), "gemma3:4b-it-qat");
}

#[tokio::test]
async fn medium_without_hints_uses_remote() {
    let local = MockProvider::new("local", "Here is a summary.");
    let remote = MockProvider::new("remote", "remote-resp");
    let health = LocalHealthChecker::seeded(true);

    let r = router(
        Arc::clone(&local),
        Arc::clone(&remote),
        health,
        RoutingHints::default(),
    );
    r.chat_with_system(None, "Summarize this", "hint:summarize", 0.7)
        .await
        .unwrap();

    assert_eq!(local.calls(), 0);
    assert_eq!(remote.calls(), 1);
}

#[tokio::test]
async fn medium_with_local_bias_hint_uses_local() {
    let local = MockProvider::new("local", "Here is a local summary.");
    let remote = MockProvider::new("remote", "remote-resp");
    let health = LocalHealthChecker::seeded(true);
    let hints = RoutingHints {
        latency_budget: crate::openhuman::routing::policy::LatencyBudget::Low,
        ..Default::default()
    };

    let r = router(Arc::clone(&local), Arc::clone(&remote), health, hints);
    r.chat_with_system(None, "Summarize this", "hint:summarize", 0.7)
        .await
        .unwrap();

    assert_eq!(local.calls(), 1);
    assert_eq!(remote.calls(), 0);
}

// ── B. Quality-based fallback ──────────────────────────────────────────

#[tokio::test]
async fn fallback_to_remote_when_local_response_low_quality() {
    let local = MockProvider::new("local", "I cannot help with that.");
    let remote = MockProvider::new("remote", "Actually here is a proper answer.");
    let health = LocalHealthChecker::seeded(true);

    let r = router(
        Arc::clone(&local),
        Arc::clone(&remote),
        health,
        RoutingHints::default(),
    );
    let result = r
        .chat_with_system(None, "react", "hint:reaction", 0.7)
        .await
        .unwrap();

    // Local returns a refusal → quality fallback → remote answer
    assert_eq!(result, "Actually here is a proper answer.");
    assert_eq!(local.calls(), 1, "local tried first");
    assert_eq!(remote.calls(), 1, "remote called on quality fallback");
}

#[tokio::test]
async fn fallback_to_remote_when_local_response_empty() {
    let local = MockProvider::new("local", "");
    let remote = MockProvider::new("remote", "Good answer from remote.");
    let health = LocalHealthChecker::seeded(true);

    let r = router(
        Arc::clone(&local),
        Arc::clone(&remote),
        health,
        RoutingHints::default(),
    );
    let result = r
        .chat_with_system(None, "classify", "hint:classify", 0.7)
        .await
        .unwrap();

    assert_eq!(result, "Good answer from remote.");
    assert_eq!(remote.calls(), 1);
}

// ── C. Error-based fallback ────────────────────────────────────────────

#[tokio::test]
async fn fallback_to_remote_when_local_errors() {
    let local = MockProvider::new("local", "never returned");
    local.set_fail(true);
    let remote = MockProvider::new("remote", "remote recovered");
    let health = LocalHealthChecker::seeded(true);

    let r = router(
        Arc::clone(&local),
        Arc::clone(&remote),
        health,
        RoutingHints::default(),
    );
    let result = r
        .chat_with_system(None, "react", "hint:reaction", 0.7)
        .await
        .unwrap();

    assert_eq!(result, "remote recovered");
    assert_eq!(local.calls(), 1);
    assert_eq!(remote.calls(), 1);
}

// ── D. Remote-only when local unhealthy ───────────────────────────────

#[tokio::test]
async fn remote_when_local_unhealthy() {
    let local = MockProvider::new("local", "never used");
    let remote = MockProvider::new("remote", "remote answer");
    let health = LocalHealthChecker::seeded(false);

    let r = router(
        Arc::clone(&local),
        Arc::clone(&remote),
        health,
        RoutingHints::default(),
    );
    r.chat_with_system(None, "react", "hint:reaction", 0.7)
        .await
        .unwrap();

    assert_eq!(local.calls(), 0, "local must not be called when unhealthy");
    assert_eq!(remote.calls(), 1);
}

// ── E. Heavy tasks always remote ──────────────────────────────────────

#[tokio::test]
async fn heavy_tasks_always_use_remote() {
    let local = MockProvider::new("local", "should not be called");
    let remote = MockProvider::new("remote", "reasoning answer");
    let health = LocalHealthChecker::seeded(true); // local is healthy

    let r = router(
        Arc::clone(&local),
        Arc::clone(&remote),
        health,
        RoutingHints::default(),
    );
    r.chat_with_system(None, "reason hard", "hint:reasoning", 0.7)
        .await
        .unwrap();

    assert_eq!(local.calls(), 0, "heavy tasks must never use local");
    assert_eq!(remote.calls(), 1);
    assert_eq!(remote.last_model(), "reasoning-v1");
}

// ── F. Privacy override ────────────────────────────────────────────────

#[tokio::test]
async fn privacy_required_never_falls_back_to_remote() {
    let local = MockProvider::new("local", "I cannot help with that.");
    local.set_fail(false); // returns low-quality, not an error
    let remote = MockProvider::new("remote", "would breach privacy");
    let health = LocalHealthChecker::seeded(true);
    let hints = RoutingHints {
        privacy_required: true,
        ..Default::default()
    };

    let r = router(Arc::clone(&local), Arc::clone(&remote), health, hints);
    // Local returns a refusal (low quality) but privacy blocks fallback.
    let result = r
        .chat_with_system(None, "private data", "hint:reaction", 0.7)
        .await
        .unwrap();

    assert!(result.contains("cannot"), "got: {result}");
    assert_eq!(
        remote.calls(),
        0,
        "remote must never be called with privacy_required"
    );
}

#[tokio::test]
async fn privacy_required_even_for_heavy_tasks() {
    // Heavy + privacy_required → still local, no remote
    let local = MockProvider::new("local", "local heavy response");
    let remote = MockProvider::new("remote", "remote");
    let health = LocalHealthChecker::seeded(true);
    let hints = RoutingHints {
        privacy_required: true,
        ..Default::default()
    };

    let r = router(Arc::clone(&local), Arc::clone(&remote), health, hints);
    r.chat_with_system(None, "reason", "hint:reasoning", 0.7)
        .await
        .unwrap();

    assert_eq!(local.calls(), 1);
    assert_eq!(remote.calls(), 0);
}

// ── G. Latency / cost hints ────────────────────────────────────────────

#[tokio::test]
async fn low_latency_hint_prefers_local() {
    let local = MockProvider::new("local", "fast local answer");
    let remote = MockProvider::new("remote", "slower remote");
    let health = LocalHealthChecker::seeded(true);
    let hints = RoutingHints {
        latency_budget: crate::openhuman::routing::policy::LatencyBudget::Low,
        ..Default::default()
    };

    let r = router(Arc::clone(&local), Arc::clone(&remote), health, hints);
    r.chat_with_system(None, "quick task", "hint:reaction", 0.7)
        .await
        .unwrap();

    assert_eq!(local.calls(), 1);
    assert_eq!(remote.calls(), 0);
}

// ── H. Integration: local disabled ────────────────────────────────────

#[tokio::test]
async fn local_disabled_all_tasks_go_remote() {
    let local = MockProvider::new("local", "should not be called");
    let remote = MockProvider::new("remote", "remote answer");
    let health = LocalHealthChecker::seeded(true);

    // Build with local_enabled = false
    let r = IntelligentRoutingProvider::new(
        Box::new(Arc::clone(&remote)),
        Box::new(Arc::clone(&local)),
        "local-model".to_string(),
        "default-remote-model".to_string(),
        false, // disabled
        health,
    );
    r.chat_with_system(None, "react", "hint:reaction", 0.7)
        .await
        .unwrap();

    assert_eq!(local.calls(), 0);
    assert_eq!(remote.calls(), 1);
}

// ── I. Regression ─────────────────────────────────────────────────────

#[tokio::test]
async fn regression_reasoning_hint_routes_remote_with_backend_model_name() {
    let local = MockProvider::new("local", "l");
    let remote = MockProvider::new("remote", "r");
    let health = LocalHealthChecker::seeded(true);

    let r = router(
        Arc::clone(&local),
        Arc::clone(&remote),
        health,
        RoutingHints::default(),
    );
    r.chat_with_system(None, "reason", "hint:reasoning", 0.7)
        .await
        .unwrap();

    // Heavy reasoning hints must be normalized to backend-valid model IDs.
    assert_eq!(remote.last_model(), "reasoning-v1");
    assert_eq!(local.calls(), 0);
}

#[tokio::test]
async fn remote_failure_propagates_without_local_fallback() {
    let local = MockProvider::new("local", "l");
    let remote = MockProvider::new("remote", "r");
    remote.set_fail(true);
    let health = LocalHealthChecker::seeded(true);

    let r = router(
        Arc::clone(&local),
        Arc::clone(&remote),
        health,
        RoutingHints::default(),
    );
    // Heavy task goes remote, remote fails → error propagates, no local retry.
    let err = r
        .chat_with_system(None, "reason", "hint:reasoning", 0.7)
        .await;
    assert!(err.is_err());
    assert_eq!(local.calls(), 0);
}

#[tokio::test]
async fn warmup_remote_failure_is_fatal_local_is_not() {
    let local = MockProvider::new("local", "l");
    local.set_fail(true);
    let remote = MockProvider::new("remote", "r");
    let health = LocalHealthChecker::seeded(true);

    let r = router(
        Arc::clone(&local),
        Arc::clone(&remote),
        health,
        RoutingHints::default(),
    );
    assert!(
        r.warmup().await.is_ok(),
        "local warmup failure must not propagate"
    );
}

#[tokio::test]
async fn capabilities_delegate_to_remote() {
    let local = MockProvider::new("local", "l");
    let remote = MockProvider::new("remote", "r");
    let health = LocalHealthChecker::seeded(true);
    let r = router(local, remote, health, RoutingHints::default());
    assert!(r.capabilities().native_tool_calling);
}
