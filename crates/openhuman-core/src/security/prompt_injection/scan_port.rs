//! Test seam for the prompt-input scan.
//!
//! In a test build the scan runs on the module-bus runtime, not the test's.
//! A `start_paused` test is idle while it waits for that answer, so its paused
//! clock auto-advances to whatever deadline the caller holds (a turn's
//! wall-clock budget, for one). A test whose subject is not the screen itself
//! installs a scripted answer here; everything around the scan (the security
//! switch, the oversize hold, verdict mapping, logging) still runs. Compiled
//! only into unit-test builds.

use std::cell::RefCell;
use std::sync::Arc;

use async_trait::async_trait;
use tinysecurity_bus::{PromptTier2, ScanPromptScoredResponse2};

/// Answers `ScanPrompt2` in place of TinySecurity.
#[async_trait]
pub(crate) trait PromptScanPort: std::fmt::Debug + Send + Sync {
    async fn scan(&self, input: String) -> Result<ScanPromptScoredResponse2, String>;
}

// Test-only, like the module itself (`mod.rs` declares it `cfg(test)`).
#[cfg(test)]
thread_local! {
    static TEST_PROMPT_SCAN_PORT: RefCell<Option<Arc<dyn PromptScanPort>>> =
        const { RefCell::new(None) };
}

/// Restores the previously installed port when dropped.
pub(crate) struct PromptScanPortGuard(Option<Arc<dyn PromptScanPort>>);

impl Drop for PromptScanPortGuard {
    fn drop(&mut self) {
        TEST_PROMPT_SCAN_PORT.with(|slot| *slot.borrow_mut() = self.0.take());
    }
}

/// Install `port` for scans made on this thread until the guard drops.
pub(crate) fn install_prompt_scan_port(port: Arc<dyn PromptScanPort>) -> PromptScanPortGuard {
    PromptScanPortGuard(TEST_PROMPT_SCAN_PORT.with(|slot| slot.borrow_mut().replace(port)))
}

/// A screen that allows every input with no rule fired.
#[derive(Debug)]
struct AllowingPromptScan;

#[async_trait]
impl PromptScanPort for AllowingPromptScan {
    async fn scan(&self, _input: String) -> Result<ScanPromptScoredResponse2, String> {
        Ok(ScanPromptScoredResponse2 {
            tier: PromptTier2::Allow,
            score_milli: 0,
            rule_ids: Vec::new(),
        })
    }
}

/// See [`AllowingPromptScan`].
pub(crate) fn install_allowing_prompt_scan() -> PromptScanPortGuard {
    install_prompt_scan_port(Arc::new(AllowingPromptScan))
}

/// The installed port's answer, or `None` to scan through TinySecurity.
pub(super) async fn scripted_scan(
    input: &str,
) -> Option<Result<ScanPromptScoredResponse2, String>> {
    let port = TEST_PROMPT_SCAN_PORT.with(|slot| slot.borrow().clone())?;
    Some(port.scan(input.to_owned()).await)
}
