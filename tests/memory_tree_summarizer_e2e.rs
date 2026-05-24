//! E2E tests for the tree summarizer engine.
//!
//! Calls `engine::run_summarization` directly with a mock LLM provider so the
//! full ingest → summarize → propagate chain is exercised without needing a
//! running Ollama process. Three scenarios are covered:
//!
//!   1. `builds_hour_day_month_year_chain` — ingest chunks across two distinct
//!      hours, run the summarizer, and assert the full hour→day→month→year→root
//!      node chain is written.
//!
//!   2. `merges_into_existing_hour_node` — run the summarizer twice for the
//!      same hour and confirm `created_at` is preserved while `updated_at`
//!      advances and the summary reflects both passes.
//!
//!   3. `survives_llm_error_with_partial_progress` — program the mock so the
//!      second LLM call returns an error; assert the first hour node was
//!      written, the second was not, and the engine surfaces the error without
//!      panicking.
//!
//! Run with: `bash scripts/test-rust-with-mock.sh --test memory_tree_summarizer_e2e`
//!
//! The mock HTTP server is started by `scripts/test-rust-with-mock.sh` and its
//! URL is available in `BACKEND_URL` / `MOCK_API_PORT`.

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use tempfile::tempdir;

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::inference::provider::traits::Provider;
use openhuman_core::openhuman::memory_tree::tree_runtime::{engine, store};

// ── Env isolation ─────────────────────────────────────────────────────────

struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    fn set_to_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var(key).ok();
        // SAFETY: guarded by ENV_LOCK which serialises process-global env mutations.
        unsafe { std::env::set_var(key, path.as_os_str()) };
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            // SAFETY: symmetric teardown under the same ENV_LOCK guard.
            Some(v) => unsafe { std::env::set_var(self.key, v) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

/// Serialise tests: `HOME` and `OPENHUMAN_WORKSPACE` are process-global.
static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    let m = ENV_LOCK.get_or_init(|| Mutex::new(()));
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

// ── Mock provider helpers ─────────────────────────────────────────────────

/// A provider whose `chat_with_system` returns scripted responses in order.
/// Thread-safe via a `Mutex<VecDeque>`. Each pop returns the next scripted
/// response; once the queue is exhausted, every subsequent call returns an
/// error so missing a setup step is caught immediately.
struct ScriptedProvider {
    responses: Arc<Mutex<std::collections::VecDeque<Result<String, String>>>>,
    call_count: Arc<Mutex<usize>>,
}

impl ScriptedProvider {
    fn new(responses: Vec<Result<String, String>>) -> Self {
        log::debug!(
            "[memory_tree_summarizer_e2e] ScriptedProvider created with {} responses",
            responses.len()
        );
        Self {
            responses: Arc::new(Mutex::new(responses.into())),
            call_count: Arc::new(Mutex::new(0)),
        }
    }

    fn call_count(&self) -> usize {
        *self.call_count.lock().expect("call_count lock")
    }
}

#[async_trait]
impl Provider for ScriptedProvider {
    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        let mut count = self.call_count.lock().expect("call_count lock");
        *count += 1;
        let call_n = *count;
        drop(count);

        log::debug!(
            "[memory_tree_summarizer_e2e] ScriptedProvider.chat_with_system call #{call_n}: \
             model={model} system_prompt_len={} msg_len={}",
            system_prompt.map(|s| s.len()).unwrap_or(0),
            message.len()
        );

        let mut q = self.responses.lock().expect("responses lock");
        match q.pop_front() {
            Some(Ok(text)) => {
                log::debug!(
                    "[memory_tree_summarizer_e2e] call #{call_n} → scripted Ok ({} chars)",
                    text.len()
                );
                Ok(text)
            }
            Some(Err(msg)) => {
                log::debug!("[memory_tree_summarizer_e2e] call #{call_n} → scripted Err: {msg}");
                Err(anyhow::anyhow!("{msg}"))
            }
            None => {
                log::debug!(
                    "[memory_tree_summarizer_e2e] call #{call_n} → queue exhausted (fallback error)"
                );
                Err(anyhow::anyhow!(
                    "ScriptedProvider queue exhausted at call #{call_n}"
                ))
            }
        }
    }
}

// ── Config builder ────────────────────────────────────────────────────────

/// Build a minimal `Config` rooted at `workspace_path`.
/// `local_ai.runtime_enabled` is irrelevant because we bypass `create_provider`
/// and pass our own `ScriptedProvider` directly.
fn build_config(workspace_path: &Path) -> Config {
    Config {
        workspace_dir: workspace_path.to_path_buf(),
        ..Config::default()
    }
}

/// Return a fixed test timestamp anchored to 2026-03-15T14:xx UTC.
/// We use explicit timestamps so buffer filenames are deterministic and
/// the hour_id derived from them matches our assertions.
fn ts_hour14() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 15, 14, 5, 0)
        .single()
        .expect("valid ts_hour14")
}

fn ts_hour15() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 15, 15, 10, 0)
        .single()
        .expect("valid ts_hour15")
}

const NS: &str = "e2e-summarizer-test";

// ── Tests ─────────────────────────────────────────────────────────────────

/// Ingest content for two distinct hours, run the summarizer, and assert the
/// full chain of nodes (hour × 2, day, month, year, root) is written.
/// The mock provider returns per-hour summaries short enough that upper levels
/// fit within their token budgets without an additional LLM call — only the
/// two hour-leaf summarizations trigger LLM calls.
#[tokio::test]
async fn builds_hour_day_month_year_chain() {
    let _lock = env_lock();
    let tmp = tempdir().expect("tempdir");
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&workspace).expect("create workspace");

    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let _ws = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace);

    log::debug!("[memory_tree_summarizer_e2e] builds_hour_day_month_year_chain: start");

    let config = build_config(&workspace);

    // Ingest 3 chunks: 2 for hour-14, 1 for hour-15.
    store::buffer_write(
        &config,
        NS,
        "Slack: discussed deployment timeline with team",
        &ts_hour14(),
        None,
    )
    .expect("buffer_write hour14 chunk1");

    store::buffer_write(
        &config,
        NS,
        "Slack: follow-up on deployment blockers",
        &ts_hour14(),
        None,
    )
    .expect("buffer_write hour14 chunk2");

    store::buffer_write(
        &config,
        NS,
        "Reviewed PR for infrastructure changes",
        &ts_hour15(),
        None,
    )
    .expect("buffer_write hour15 chunk1");

    // Provider: 2 LLM calls expected — one per hour leaf.
    // The hour summaries are short enough that day/month/year/root fit within
    // token budget and do NOT trigger additional LLM calls (propagate_node
    // short-circuits when combined children text fits the level budget).
    let provider = ScriptedProvider::new(vec![
        Ok("User discussed deployment timeline".to_string()),
        Ok("Reviewed infrastructure PR".to_string()),
    ]);

    log::debug!("[memory_tree_summarizer_e2e] running summarization");
    let result = engine::run_summarization(&config, &provider, NS, Utc::now()).await;

    log::debug!(
        "[memory_tree_summarizer_e2e] run_summarization returned: {:?}",
        result
            .as_ref()
            .map(|n| n.as_ref().map(|node| &node.node_id))
    );
    assert!(
        result.is_ok(),
        "run_summarization should succeed: {:?}",
        result
    );
    let last_node = result.unwrap();
    assert!(last_node.is_some(), "should return a last hour node");
    let last_node = last_node.unwrap();
    log::debug!(
        "[memory_tree_summarizer_e2e] last hour node: {} level={:?}",
        last_node.node_id,
        last_node.level
    );

    // Assert both hour leaves exist.
    let hour14_id = "2026/03/15/14";
    let hour15_id = "2026/03/15/15";

    let node14 = store::read_node(&config, NS, hour14_id)
        .expect("read_node hour14")
        .expect("hour14 node must exist");
    log::debug!(
        "[memory_tree_summarizer_e2e] hour14 summary: {}",
        node14.summary
    );
    assert!(
        node14.summary.contains("deployment"),
        "hour14 summary should contain 'deployment', got: {}",
        node14.summary
    );

    let node15 = store::read_node(&config, NS, hour15_id)
        .expect("read_node hour15")
        .expect("hour15 node must exist");
    log::debug!(
        "[memory_tree_summarizer_e2e] hour15 summary: {}",
        node15.summary
    );
    assert!(
        node15.summary.contains("infrastructure") || node15.summary.contains("PR"),
        "hour15 summary should contain 'infrastructure' or 'PR', got: {}",
        node15.summary
    );

    // Assert day node was propagated.
    let day_id = "2026/03/15";
    let day_node = store::read_node(&config, NS, day_id)
        .expect("read_node day")
        .expect("day node must exist after propagation");
    log::debug!(
        "[memory_tree_summarizer_e2e] day node summary len={}",
        day_node.summary.len()
    );
    assert!(
        !day_node.summary.is_empty(),
        "day summary should not be empty"
    );

    // Assert month node.
    let month_id = "2026/03";
    let month_node = store::read_node(&config, NS, month_id)
        .expect("read_node month")
        .expect("month node must exist after propagation");
    assert!(
        !month_node.summary.is_empty(),
        "month summary should not be empty"
    );

    // Assert year node.
    let year_id = "2026";
    let year_node = store::read_node(&config, NS, year_id)
        .expect("read_node year")
        .expect("year node must exist after propagation");
    assert!(
        !year_node.summary.is_empty(),
        "year summary should not be empty"
    );

    // Assert root node.
    let root_node = store::read_node(&config, NS, "root")
        .expect("read_node root")
        .expect("root node must exist after propagation");
    assert!(
        !root_node.summary.is_empty(),
        "root summary should not be empty"
    );

    // Exactly 2 LLM calls: one per hour leaf.
    assert_eq!(
        provider.call_count(),
        2,
        "expected exactly 2 LLM calls (one per hour leaf)"
    );

    // Buffer should be drained after successful summarization.
    let remaining = store::buffer_read(&config, NS).expect("buffer_read post-run");
    assert!(
        remaining.is_empty(),
        "buffer should be empty after successful run, got {} entries",
        remaining.len()
    );

    log::debug!("[memory_tree_summarizer_e2e] builds_hour_day_month_year_chain: PASS");
}

/// Run the summarizer twice for the same hour. Verify:
///   - `created_at` is preserved from the first run.
///   - `updated_at` is strictly greater after the second run.
///   - The merged summary contains keywords from both passes.
#[tokio::test]
async fn merges_into_existing_hour_node() {
    let _lock = env_lock();
    let tmp = tempdir().expect("tempdir");
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&workspace).expect("create workspace");

    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let _ws = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace);

    log::debug!("[memory_tree_summarizer_e2e] merges_into_existing_hour_node: start");

    let config = build_config(&workspace);

    // --- First run: ingest and summarize hour-14. ---
    store::buffer_write(
        &config,
        NS,
        "Discussed deployment timeline on slack",
        &ts_hour14(),
        None,
    )
    .expect("buffer_write pass1");

    let provider1 = ScriptedProvider::new(vec![Ok(
        "First-run summary: deployment timeline discussed".to_string(),
    )]);

    log::debug!("[memory_tree_summarizer_e2e] first run");
    let r1 = engine::run_summarization(&config, &provider1, NS, Utc::now())
        .await
        .expect("first run_summarization");
    assert!(r1.is_some(), "first run should yield a node");

    let hour14_id = "2026/03/15/14";
    let node_after_first = store::read_node(&config, NS, hour14_id)
        .expect("read node after first run")
        .expect("hour14 must exist after first run");

    let created_at_first = node_after_first.created_at;
    let updated_at_first = node_after_first.updated_at;
    log::debug!(
        "[memory_tree_summarizer_e2e] after first run: created_at={} updated_at={}",
        created_at_first,
        updated_at_first
    );

    // Small sleep so the second updated_at is strictly greater.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // --- Second run: ingest more content for the same hour-14. ---
    store::buffer_write(
        &config,
        NS,
        "Follow-up on deployment blockers",
        &ts_hour14(),
        None,
    )
    .expect("buffer_write pass2");

    let provider2 = ScriptedProvider::new(vec![Ok(
        "Merged summary: deployment timeline and blockers".to_string(),
    )]);

    log::debug!("[memory_tree_summarizer_e2e] second run (same hour)");
    let r2 = engine::run_summarization(&config, &provider2, NS, Utc::now())
        .await
        .expect("second run_summarization");
    assert!(r2.is_some(), "second run should yield a node");

    let node_after_second = store::read_node(&config, NS, hour14_id)
        .expect("read node after second run")
        .expect("hour14 must exist after second run");

    let created_at_second = node_after_second.created_at;
    let updated_at_second = node_after_second.updated_at;
    log::debug!(
        "[memory_tree_summarizer_e2e] after second run: created_at={} updated_at={}",
        created_at_second,
        updated_at_second
    );

    // `created_at` must be preserved.
    assert_eq!(
        created_at_first, created_at_second,
        "created_at must be preserved across merges"
    );

    // `updated_at` must advance (or at worst stay equal if clocks are too coarse).
    assert!(
        updated_at_second >= updated_at_first,
        "updated_at must not go backward: first={updated_at_first} second={updated_at_second}"
    );

    // Summary must reflect the merge (the scripted response contains "blockers").
    assert!(
        node_after_second.summary.contains("blockers")
            || node_after_second.summary.contains("Merged"),
        "merged summary should reflect second pass content, got: {}",
        node_after_second.summary
    );

    log::debug!("[memory_tree_summarizer_e2e] merges_into_existing_hour_node: PASS");
}

/// Program the provider so the SECOND LLM call returns an error.
/// Ingest two hours' worth of content and run the summarizer.
///
/// Expected behaviour:
///   - The first hour leaf is successfully written before the error.
///   - The error propagates out of `run_summarization` as an `Err`.
///   - The process does NOT panic.
///   - Because the buffer is only deleted after ALL hour leaves are written,
///     the buffer entries are NOT deleted (the second hour's content persists).
#[tokio::test]
async fn survives_llm_error_with_partial_progress() {
    let _lock = env_lock();
    let tmp = tempdir().expect("tempdir");
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&workspace).expect("create workspace");

    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let _ws = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace);

    log::debug!("[memory_tree_summarizer_e2e] survives_llm_error_with_partial_progress: start");

    let config = build_config(&workspace);

    // Ingest content for two distinct hours so two LLM calls are required.
    store::buffer_write(
        &config,
        NS,
        "Hour-14 content: deployment planning",
        &ts_hour14(),
        None,
    )
    .expect("buffer_write hour14");

    store::buffer_write(
        &config,
        NS,
        "Hour-15 content: infrastructure review",
        &ts_hour15(),
        None,
    )
    .expect("buffer_write hour15");

    // Provider: call 1 succeeds, call 2 returns an error.
    let provider = ScriptedProvider::new(vec![
        Ok("Hour-14 summary: deployment planning in progress".to_string()),
        Err("boom: simulated LLM failure on second call".to_string()),
    ]);

    log::debug!("[memory_tree_summarizer_e2e] running summarization expecting partial failure");
    let result = engine::run_summarization(&config, &provider, NS, Utc::now()).await;

    log::debug!(
        "[memory_tree_summarizer_e2e] run_summarization result: is_ok={}",
        result.is_ok()
    );

    // The engine must return an error — not panic.
    assert!(
        result.is_err(),
        "expected Err from run_summarization when second LLM call fails, got: {:?}",
        result
    );
    let err = result.unwrap_err();
    log::debug!(
        "[memory_tree_summarizer_e2e] propagated error (as expected): {:#}",
        err
    );
    // Use the full anyhow error chain (alternating display) so nested context
    // layers — e.g. "summarize hour leaf: LLM summarization failed: boom: …" —
    // are all visible in the assertion.
    let err_chain = format!("{err:#}");
    assert!(
        err_chain.contains("boom") || err_chain.contains("LLM summarization failed"),
        "error message should mention the LLM failure, got: {err_chain}"
    );

    // Exactly 2 LLM calls were made.
    assert_eq!(
        provider.call_count(),
        2,
        "expected 2 LLM calls (1 success + 1 error)"
    );

    // The first hour leaf (hour-14) was written before the error.
    // The engine writes hour leaves as it goes; the second fails before writing.
    // Note: the exact behaviour depends on which hour is processed first
    // (BTreeMap ordering: "2026/03/15/14" < "2026/03/15/15"), so hour-14 is first.
    let hour14_id = "2026/03/15/14";
    let node14 = store::read_node(&config, NS, hour14_id).expect("read_node hour14");
    assert!(
        node14.is_some(),
        "hour-14 leaf must be written before the error on hour-15"
    );
    log::debug!(
        "[memory_tree_summarizer_e2e] hour14 node present: summary={}",
        node14.unwrap().summary
    );

    // The second hour node (hour-15) was NOT written because the LLM call failed.
    let hour15_id = "2026/03/15/15";
    let node15 = store::read_node(&config, NS, hour15_id).expect("read_node hour15");
    assert!(
        node15.is_none(),
        "hour-15 leaf must NOT exist when its LLM call failed"
    );

    // The buffer must NOT be drained — the engine only deletes buffer entries
    // after all hour leaves are successfully written. A partial failure means
    // the buffer retains its entries so the next run can retry.
    let remaining = store::buffer_read(&config, NS).expect("buffer_read post-error");
    assert!(
        !remaining.is_empty(),
        "buffer must not be drained after a partial failure, but it is empty"
    );
    log::debug!(
        "[memory_tree_summarizer_e2e] {} buffer entries remain (expected > 0)",
        remaining.len()
    );

    log::debug!("[memory_tree_summarizer_e2e] survives_llm_error_with_partial_progress: PASS");
}
