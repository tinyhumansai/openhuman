use super::*;
use crate::openhuman::memory::api::error::MemoryError;
use crate::openhuman::memory::api::provider::retrieval::RetrievalResponse;
use crate::openhuman::memory::api::types::NamespaceMemoryHit;
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Scripted {
    outcome: Result<(), String>,
    delay: Duration,
    calls: AtomicUsize,
}

impl Scripted {
    fn ok() -> Self {
        Self {
            outcome: Ok(()),
            delay: Duration::ZERO,
            calls: AtomicUsize::new(0),
        }
    }

    fn failing() -> Self {
        Self {
            outcome: Err("signed out".into()),
            delay: Duration::ZERO,
            calls: AtomicUsize::new(0),
        }
    }

    fn slow(delay: Duration) -> Self {
        Self {
            outcome: Ok(()),
            delay,
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl AutoRecallSource for Scripted {
    async fn fast_retrieve(
        &self,
        query: &str,
        options: FastRetrieveQuery,
    ) -> Result<RetrievalResponse, MemoryError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(query, WARM_UP_QUERY);
        assert_eq!(options.limit, 1, "the warm-up asks for the smallest page");
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        match &self.outcome {
            Ok(()) => Ok(RetrievalResponse::default()),
            Err(message) => Err(MemoryError::Backend(message.clone())),
        }
    }

    async fn recall_namespace_scored(
        &self,
        _namespace: &str,
        _query: &str,
        _limit: usize,
    ) -> Result<Vec<NamespaceMemoryHit>, MemoryError> {
        // The warm-up is one tree retrieval: that is what loads the NLP server
        // and the embedder, and the notes leg needs only the second of those.
        panic!("the warm-up must not read the notes namespace");
    }
}

#[tokio::test]
async fn warm_up_runs_one_bounded_retrieval() {
    let source = Scripted::ok();
    let outcome = warm_up(&source, Duration::from_secs(1)).await;
    assert!(
        matches!(outcome, WarmUpOutcome::Warmed { .. }),
        "{outcome:?}"
    );
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn warm_up_reports_a_failure_without_retrying() {
    let source = Scripted::failing();
    let outcome = warm_up(&source, Duration::from_secs(1)).await;
    assert!(
        matches!(outcome, WarmUpOutcome::Failed { .. }),
        "{outcome:?}"
    );
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn warm_up_gives_up_when_the_budget_expires() {
    let source = Scripted::slow(Duration::from_millis(200));
    let outcome = warm_up(&source, Duration::from_millis(20)).await;
    assert_eq!(outcome, WarmUpOutcome::TimedOut);
}

#[tokio::test]
async fn spawn_at_boot_is_a_no_op_when_the_lane_is_off() {
    let mut config = Config::default();
    config.subsystems.memory.hooks.auto_recall = false;
    // Nothing to observe but "does not panic and does not block": the spawn
    // never happens, so there is no task to wait for.
    spawn_at_boot(Arc::new(config));
}

/// The boot path end to end against whatever driver the test build binds
/// (the null driver unless a module is staged): binding succeeds, the guard
/// has no retrieval family, the warm-up reports `Warmed` on an empty page.
/// Whatever the driver, the contract is the same — no error, one log line.
#[tokio::test]
async fn warm_up_from_config_binds_and_degrades_quietly() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();

    let outcome = warm_up_from_config(&config).await;
    assert!(
        !matches!(outcome, Some(WarmUpOutcome::TimedOut)),
        "a warm-up against the test driver must not hang: {outcome:?}"
    );
}
