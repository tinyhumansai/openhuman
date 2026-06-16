# AgentBox Marketplace Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `openhuman-core` as an AgentBox marketplace container by adding `POST /run` + `GET /jobs/{job_id}` HTTP endpoints that drive the full agent runtime, plus a GMI MaaS env-var bridge to the existing OpenAI-compatible provider.

**Architecture:** New domain `src/openhuman/agentbox/` containing an Axum sub-router, an in-memory job store, an `AgentInvoker` abstraction for the agent runtime, and a GMI provider bridge. Mounted into the existing core HTTP router behind `OPENHUMAN_AGENTBOX_MODE=1`. No persistence, no streaming (AgentBox is polling-only). Desktop builds unaffected.

**Tech Stack:** Rust 2021, Axum 0.8, Tokio, `parking_lot::RwLock`, `uuid` v4, the existing `compatible_provider_impl` OpenAI-compatible client.

**Spec:** `docs/superpowers/specs/2026-06-12-agentbox-marketplace-integration-design.md`

---

## File Structure

| Path                                                  | Created / Modified | Responsibility                                                              |
| ----------------------------------------------------- | ------------------ | --------------------------------------------------------------------------- |
| `src/openhuman/agentbox/mod.rs`                       | Create             | Module wiring, `pub use` re-exports. No logic.                              |
| `src/openhuman/agentbox/types.rs`                     | Create             | Serde request/response/job types.                                           |
| `src/openhuman/agentbox/store.rs`                     | Create             | `JobStore` — concurrent map + sweep.                                        |
| `src/openhuman/agentbox/invoker.rs`                   | Create             | `AgentInvoker` trait + production impl bridging to the agent runtime.       |
| `src/openhuman/agentbox/ops.rs`                       | Create             | `submit_run`, `get_job`, `run_job` worker.                                  |
| `src/openhuman/agentbox/http.rs`                      | Create             | Axum sub-router + handlers.                                                 |
| `src/openhuman/agentbox/env.rs`                       | Create             | `register_gmi_provider_if_present()` startup hook.                          |
| `src/openhuman/agentbox/store_tests.rs`               | Create             | Store unit tests.                                                           |
| `src/openhuman/agentbox/ops_tests.rs`                 | Create             | Ops unit tests (with mock invoker).                                         |
| `src/openhuman/agentbox/http_tests.rs`                | Create             | HTTP integration tests via `tower::ServiceExt`.                             |
| `src/openhuman/agentbox/env_tests.rs`                 | Create             | Env bridge unit tests.                                                      |
| `src/openhuman/mod.rs`                                | Modify             | Add `pub mod agentbox;`.                                                    |
| `src/core/auth.rs`                                    | Modify             | Add `/run` and `/jobs/` prefix bypass.                                      |
| `src/core/jsonrpc.rs`                                 | Modify             | Mount AgentBox sub-router behind env flag; call GMI bridge at startup.      |
| `Dockerfile`                                          | Modify             | Add `ENV OPENHUMAN_AGENTBOX_MODE=0` default.                                |
| `.env.example`                                        | Modify             | Document the five new env vars.                                             |
| `gitbooks/developing/agentbox-deployment.md`          | Create             | Operator runbook.                                                           |
| `tests/agentbox_e2e.rs`                               | Create             | Cross-binary end-to-end check.                                              |

Files modified later in the plan (Tasks 11–14) are listed for visibility but produce no code requiring TDD beyond what the earlier tasks already validate.

---

## Task 1: Module skeleton + types

**Files:**
- Create: `src/openhuman/agentbox/mod.rs`
- Create: `src/openhuman/agentbox/types.rs`
- Modify: `src/openhuman/mod.rs` (add `pub mod agentbox;`)

- [ ] **Step 1.1: Create the module file**

Create `src/openhuman/agentbox/mod.rs`:

```rust
//! AgentBox marketplace adapter.
//!
//! Exposes `POST /run` and `GET /jobs/{job_id}` over the existing core HTTP
//! server when `OPENHUMAN_AGENTBOX_MODE=1`. Each `/run` invocation drives the
//! full agent runtime; the result is polled via `/jobs/{job_id}`.
//!
//! See `docs/superpowers/specs/2026-06-12-agentbox-marketplace-integration-design.md`.

pub mod env;
pub mod http;
pub mod invoker;
pub mod ops;
pub mod store;
pub mod types;

pub use env::register_gmi_provider_if_present;
pub use http::router as agentbox_router;
pub use store::JobStore;

#[cfg(test)]
mod env_tests;
#[cfg(test)]
mod http_tests;
#[cfg(test)]
mod ops_tests;
#[cfg(test)]
mod store_tests;
```

- [ ] **Step 1.2: Create types**

Create `src/openhuman/agentbox/types.rs`:

```rust
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Wire-format request: `POST /run` body.
#[derive(Debug, Clone, Deserialize)]
pub struct RunRequest {
    pub payload: RunPayload,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunPayload {
    pub message: String,
    #[serde(default)]
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunResponse {
    pub job_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RunResult {
    pub message: String,
    pub thread_id: String,
}

/// Wire-format response: `GET /jobs/{job_id}` body.
#[derive(Debug, Clone, Serialize)]
pub struct JobView {
    pub status: JobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<RunResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Internal store record.
#[derive(Debug, Clone)]
pub struct JobRecord {
    pub status: JobStatus,
    pub result: Option<RunResult>,
    pub error: Option<String>,
    pub created_at: Instant,
    pub terminal_at: Option<Instant>,
}

impl JobRecord {
    pub fn new_pending() -> Self {
        Self {
            status: JobStatus::Pending,
            result: None,
            error: None,
            created_at: Instant::now(),
            terminal_at: None,
        }
    }

    pub fn view(&self) -> JobView {
        JobView {
            status: self.status,
            result: self.result.clone(),
            error: self.error.clone(),
        }
    }
}
```

- [ ] **Step 1.3: Register the module**

Edit `src/openhuman/mod.rs` — find the alphabetically-sorted `pub mod` list and insert `pub mod agentbox;` immediately after `pub mod about_app;` (or wherever alphabetical order places it; current order has `agent`, `agent_experience`, etc., so the new entry goes between `about_app` and `accessibility`).

```rust
pub mod about_app;
pub mod accessibility;
pub mod agent;
pub mod agent_experience;
// ... existing modules ...

// add (in alphabetical order, between accessibility and agent):
pub mod agentbox;
```

Note: the precise position is between `accessibility` and `agent` because `agentbox` sorts after `agent_*` would imply, but `agentbox` < `agent_e` is false; sort by full identifier. Verify by running `cargo check` after Step 1.4 — a misplaced entry produces no error, just convention drift.

- [ ] **Step 1.4: Verify it compiles**

```bash
cargo check --manifest-path Cargo.toml --bin openhuman-core
```

Expected: OK (warnings about unused modules are fine — we'll wire them up in later tasks).

- [ ] **Step 1.5: Commit**

```bash
git add src/openhuman/agentbox/mod.rs src/openhuman/agentbox/types.rs src/openhuman/mod.rs
git commit -m "feat(agentbox): scaffold module and wire types (#3620)"
```

---

## Task 2: Job store (TDD)

**Files:**
- Create: `src/openhuman/agentbox/store.rs`
- Create: `src/openhuman/agentbox/store_tests.rs`

- [ ] **Step 2.1: Write failing tests**

Create `src/openhuman/agentbox/store_tests.rs`:

```rust
use super::store::JobStore;
use super::types::{JobRecord, JobStatus, RunResult};
use std::time::Duration;

#[test]
fn insert_and_get_round_trip() {
    let store = JobStore::new(Duration::from_secs(3600));
    let id = store.insert_pending();
    let view = store.get(&id).expect("just inserted");
    assert_eq!(view.status, JobStatus::Pending);
    assert!(view.result.is_none());
    assert!(view.error.is_none());
}

#[test]
fn get_unknown_returns_none() {
    let store = JobStore::new(Duration::from_secs(3600));
    assert!(store.get("nope").is_none());
}

#[test]
fn mark_completed_sets_status_result_and_terminal_at() {
    let store = JobStore::new(Duration::from_secs(3600));
    let id = store.insert_pending();
    let result = RunResult {
        message: "hi".into(),
        thread_id: "t-1".into(),
    };
    store.mark_completed(&id, result.clone());
    let view = store.get(&id).unwrap();
    assert_eq!(view.status, JobStatus::Completed);
    assert_eq!(view.result, Some(result));
    assert!(view.error.is_none());
}

#[test]
fn mark_failed_sets_status_and_error() {
    let store = JobStore::new(Duration::from_secs(3600));
    let id = store.insert_pending();
    store.mark_failed(&id, "boom".into());
    let view = store.get(&id).unwrap();
    assert_eq!(view.status, JobStatus::Failed);
    assert!(view.result.is_none());
    assert_eq!(view.error.as_deref(), Some("boom"));
}

#[test]
fn mark_running_sets_status_only() {
    let store = JobStore::new(Duration::from_secs(3600));
    let id = store.insert_pending();
    store.mark_running(&id);
    let view = store.get(&id).unwrap();
    assert_eq!(view.status, JobStatus::Running);
}

#[test]
fn sweep_evicts_terminal_jobs_older_than_retention() {
    // Retention=0 means any terminal job is immediately sweepable.
    let store = JobStore::new(Duration::from_secs(0));
    let id_done = store.insert_pending();
    store.mark_completed(
        &id_done,
        RunResult {
            message: "".into(),
            thread_id: "t".into(),
        },
    );
    let id_running = store.insert_pending();
    store.mark_running(&id_running);

    let evicted = store.sweep_now();

    assert_eq!(evicted, 1);
    assert!(store.get(&id_done).is_none(), "terminal job evicted");
    assert!(store.get(&id_running).is_some(), "running job retained");
}

#[test]
fn sweep_leaves_recent_terminal_jobs() {
    let store = JobStore::new(Duration::from_secs(3600));
    let id = store.insert_pending();
    store.mark_completed(
        &id,
        RunResult {
            message: "".into(),
            thread_id: "t".into(),
        },
    );
    assert_eq!(store.sweep_now(), 0);
    assert!(store.get(&id).is_some());
}

#[test]
fn insert_pending_returns_uuid_v4_format() {
    let store = JobStore::new(Duration::from_secs(3600));
    let id = store.insert_pending();
    // v4 UUIDs are 36 chars (32 hex + 4 dashes).
    assert_eq!(id.len(), 36, "uuid v4 string length");
    assert_eq!(id.chars().filter(|c| *c == '-').count(), 4);
}
```

- [ ] **Step 2.2: Run tests — expect compile failure**

```bash
cargo test --manifest-path Cargo.toml --lib openhuman::agentbox::store 2>&1 | tail -20
```

Expected: build errors — `JobStore` not defined.

- [ ] **Step 2.3: Implement the store**

Create `src/openhuman/agentbox/store.rs`:

```rust
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

use super::types::{JobRecord, JobStatus, JobView, RunResult};

/// Thread-safe in-memory job store with terminal-job retention sweeping.
///
/// Jobs are kept until `retention` has elapsed past their `terminal_at`.
/// Running and pending jobs are never evicted.
#[derive(Clone)]
pub struct JobStore {
    inner: Arc<RwLock<HashMap<String, JobRecord>>>,
    retention: Duration,
}

impl JobStore {
    pub fn new(retention: Duration) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            retention,
        }
    }

    pub fn insert_pending(&self) -> String {
        let id = Uuid::new_v4().to_string();
        self.inner.write().insert(id.clone(), JobRecord::new_pending());
        id
    }

    pub fn get(&self, id: &str) -> Option<JobView> {
        self.inner.read().get(id).map(|r| r.view())
    }

    pub fn mark_running(&self, id: &str) {
        if let Some(rec) = self.inner.write().get_mut(id) {
            rec.status = JobStatus::Running;
        }
    }

    pub fn mark_completed(&self, id: &str, result: RunResult) {
        if let Some(rec) = self.inner.write().get_mut(id) {
            rec.status = JobStatus::Completed;
            rec.result = Some(result);
            rec.error = None;
            rec.terminal_at = Some(Instant::now());
        }
    }

    pub fn mark_failed(&self, id: &str, error: String) {
        if let Some(rec) = self.inner.write().get_mut(id) {
            rec.status = JobStatus::Failed;
            rec.result = None;
            rec.error = Some(error);
            rec.terminal_at = Some(Instant::now());
        }
    }

    /// Evict terminal jobs whose `terminal_at` is older than the retention
    /// window. Returns the number of jobs removed.
    pub fn sweep_now(&self) -> usize {
        let now = Instant::now();
        let retention = self.retention;
        let mut guard = self.inner.write();
        let before = guard.len();
        guard.retain(|_, rec| match rec.terminal_at {
            Some(t) => now.duration_since(t) < retention,
            None => true,
        });
        before - guard.len()
    }
}
```

- [ ] **Step 2.4: Run tests — expect pass**

```bash
cargo test --manifest-path Cargo.toml --lib openhuman::agentbox::store 2>&1 | tail -15
```

Expected: 8 passed.

- [ ] **Step 2.5: Commit**

```bash
git add src/openhuman/agentbox/store.rs src/openhuman/agentbox/store_tests.rs
git commit -m "feat(agentbox): in-memory job store with TTL sweep (#3620)"
```

---

## Task 3: AgentInvoker abstraction (no real wiring yet)

**Files:**
- Create: `src/openhuman/agentbox/invoker.rs`

We need a trait so the rest of the plan can test against a mock. The production wiring lands in Task 9.

- [ ] **Step 3.1: Create the trait + stub production impl**

Create `src/openhuman/agentbox/invoker.rs`:

```rust
use async_trait::async_trait;
use std::sync::Arc;

/// Bridges AgentBox `/run` invocations to OpenHuman's agent runtime.
///
/// Implementations resolve (or create) a thread, drive a single user turn
/// through the full agent runtime (skills, tools, memory), and return the
/// final assistant text + the thread id used.
#[async_trait]
pub trait AgentInvoker: Send + Sync + 'static {
    async fn invoke(
        &self,
        thread_id: Option<&str>,
        message: &str,
    ) -> Result<InvocationOutput, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationOutput {
    pub assistant_message: String,
    pub thread_id: String,
}

/// Production impl — wired to the real agent runtime in Task 9.
///
/// This is intentionally a stub today so the surrounding HTTP + store layers
/// can land and be tested independently. The stub fails loudly so a wrongly
/// deployed early build cannot silently no-op.
#[derive(Default)]
pub struct CoreAgentInvoker;

#[async_trait]
impl AgentInvoker for CoreAgentInvoker {
    async fn invoke(
        &self,
        _thread_id: Option<&str>,
        _message: &str,
    ) -> Result<InvocationOutput, String> {
        Err("agentbox: agent runtime bridge not wired (Task 9)".into())
    }
}

/// Convenience alias used by the rest of the module.
pub type SharedInvoker = Arc<dyn AgentInvoker>;
```

- [ ] **Step 3.2: Verify it compiles**

```bash
cargo check --manifest-path Cargo.toml --bin openhuman-core 2>&1 | tail -10
```

Expected: OK. The `async_trait` crate is already a dependency (used widely in the codebase — confirm with `grep async_trait Cargo.toml` if needed).

- [ ] **Step 3.3: Commit**

```bash
git add src/openhuman/agentbox/invoker.rs
git commit -m "feat(agentbox): add AgentInvoker trait + stub production impl (#3620)"
```

---

## Task 4: Ops (`submit_run`, `get_job`, `run_job` worker) — TDD with mock invoker

**Files:**
- Create: `src/openhuman/agentbox/ops.rs`
- Create: `src/openhuman/agentbox/ops_tests.rs`

- [ ] **Step 4.1: Write failing tests**

Create `src/openhuman/agentbox/ops_tests.rs`:

```rust
use super::invoker::{AgentInvoker, InvocationOutput};
use super::ops::{run_job, submit_run};
use super::store::JobStore;
use super::types::{JobStatus, RunPayload};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

struct StaticInvoker {
    response: Result<InvocationOutput, String>,
}

#[async_trait]
impl AgentInvoker for StaticInvoker {
    async fn invoke(
        &self,
        _thread_id: Option<&str>,
        _message: &str,
    ) -> Result<InvocationOutput, String> {
        self.response.clone()
    }
}

struct BlockingInvoker {
    gate: Arc<Notify>,
}

#[async_trait]
impl AgentInvoker for BlockingInvoker {
    async fn invoke(
        &self,
        _thread_id: Option<&str>,
        _message: &str,
    ) -> Result<InvocationOutput, String> {
        // Block until the test releases us — used to assert running status.
        self.gate.notified().await;
        Ok(InvocationOutput {
            assistant_message: "released".into(),
            thread_id: "t".into(),
        })
    }
}

#[tokio::test]
async fn submit_run_returns_pending_job_immediately() {
    let store = JobStore::new(Duration::from_secs(3600));
    let invoker = Arc::new(BlockingInvoker {
        gate: Arc::new(Notify::new()),
    });
    let id = submit_run(
        store.clone(),
        invoker,
        RunPayload {
            message: "hi".into(),
            thread_id: None,
        },
        Duration::from_secs(60),
    );
    let view = store.get(&id).expect("inserted");
    // Status is Pending or Running depending on scheduling — both are fine.
    assert!(matches!(
        view.status,
        JobStatus::Pending | JobStatus::Running
    ));
}

#[tokio::test]
async fn run_job_happy_path_marks_completed_with_message() {
    let store = JobStore::new(Duration::from_secs(3600));
    let invoker = Arc::new(StaticInvoker {
        response: Ok(InvocationOutput {
            assistant_message: "hello, world".into(),
            thread_id: "t-42".into(),
        }),
    });
    let id = store.insert_pending();
    run_job(
        store.clone(),
        invoker,
        id.clone(),
        RunPayload {
            message: "ping".into(),
            thread_id: None,
        },
        Duration::from_secs(5),
    )
    .await;
    let view = store.get(&id).unwrap();
    assert_eq!(view.status, JobStatus::Completed);
    let res = view.result.unwrap();
    assert_eq!(res.message, "hello, world");
    assert_eq!(res.thread_id, "t-42");
}

#[tokio::test]
async fn run_job_invoker_error_marks_failed() {
    let store = JobStore::new(Duration::from_secs(3600));
    let invoker = Arc::new(StaticInvoker {
        response: Err("upstream down".into()),
    });
    let id = store.insert_pending();
    run_job(
        store.clone(),
        invoker,
        id.clone(),
        RunPayload {
            message: "ping".into(),
            thread_id: None,
        },
        Duration::from_secs(5),
    )
    .await;
    let view = store.get(&id).unwrap();
    assert_eq!(view.status, JobStatus::Failed);
    assert_eq!(view.error.as_deref(), Some("upstream down"));
    assert!(view.result.is_none());
}

#[tokio::test]
async fn run_job_timeout_marks_failed_with_timeout_message() {
    let store = JobStore::new(Duration::from_secs(3600));
    let gate = Arc::new(Notify::new());
    let invoker = Arc::new(BlockingInvoker { gate });
    let id = store.insert_pending();
    run_job(
        store.clone(),
        invoker,
        id.clone(),
        RunPayload {
            message: "ping".into(),
            thread_id: None,
        },
        Duration::from_millis(20),
    )
    .await;
    let view = store.get(&id).unwrap();
    assert_eq!(view.status, JobStatus::Failed);
    let err = view.error.unwrap();
    assert!(
        err.contains("timeout"),
        "expected timeout error, got: {err}"
    );
}
```

- [ ] **Step 4.2: Run tests — expect compile failure**

```bash
cargo test --manifest-path Cargo.toml --lib openhuman::agentbox::ops 2>&1 | tail -10
```

Expected: `submit_run`/`run_job` not defined.

- [ ] **Step 4.3: Implement ops**

Create `src/openhuman/agentbox/ops.rs`:

```rust
use std::time::Duration;
use tokio::time::timeout;

use super::invoker::SharedInvoker;
use super::store::JobStore;
use super::types::{RunPayload, RunResult};

/// Spawn a worker for `payload` and return the new job id immediately.
///
/// Caller-visible behavior: status is `pending` for the brief window before
/// the worker task is scheduled, then transitions to `running` and finally
/// `completed` / `failed`.
pub fn submit_run(
    store: JobStore,
    invoker: SharedInvoker,
    payload: RunPayload,
    job_timeout: Duration,
) -> String {
    let id = store.insert_pending();
    let id_clone = id.clone();
    tokio::spawn(async move {
        run_job(store, invoker, id_clone, payload, job_timeout).await;
    });
    id
}

/// Run a single job synchronously inside the calling task.
///
/// Public so tests can drive it without `tokio::spawn` indirection.
pub async fn run_job(
    store: JobStore,
    invoker: SharedInvoker,
    job_id: String,
    payload: RunPayload,
    job_timeout: Duration,
) {
    store.mark_running(&job_id);
    let message = payload.message;
    let thread_id = payload.thread_id;

    let invocation = invoker.invoke(thread_id.as_deref(), &message);
    let outcome = timeout(job_timeout, invocation).await;

    match outcome {
        Ok(Ok(output)) => {
            log::info!(
                "[agentbox] job {} completed thread_id={} reply_len={}",
                job_id,
                output.thread_id,
                output.assistant_message.len()
            );
            store.mark_completed(
                &job_id,
                RunResult {
                    message: output.assistant_message,
                    thread_id: output.thread_id,
                },
            );
        }
        Ok(Err(err)) => {
            log::warn!("[agentbox] job {} failed: {}", job_id, err);
            store.mark_failed(&job_id, err);
        }
        Err(_elapsed) => {
            let secs = job_timeout.as_secs();
            let msg = format!("job timeout after {}s", secs);
            log::warn!("[agentbox] job {} {}", job_id, msg);
            store.mark_failed(&job_id, msg);
        }
    }
}
```

- [ ] **Step 4.4: Run tests — expect pass**

```bash
cargo test --manifest-path Cargo.toml --lib openhuman::agentbox::ops 2>&1 | tail -10
```

Expected: 4 passed.

- [ ] **Step 4.5: Commit**

```bash
git add src/openhuman/agentbox/ops.rs src/openhuman/agentbox/ops_tests.rs
git commit -m "feat(agentbox): submit_run + run_job worker with timeout (#3620)"
```

---

## Task 5: HTTP layer — `POST /run` (TDD)

**Files:**
- Create: `src/openhuman/agentbox/http.rs`
- Create: `src/openhuman/agentbox/http_tests.rs`

We split the HTTP work into two tasks (Task 5 for `/run`, Task 6 for `/jobs/{id}`) so each commit is bite-sized.

- [ ] **Step 5.1: Write failing tests for POST /run**

Create `src/openhuman/agentbox/http_tests.rs`:

```rust
use super::http::router;
use super::invoker::{AgentInvoker, InvocationOutput};
use super::store::JobStore;
use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

struct EchoInvoker;

#[async_trait]
impl AgentInvoker for EchoInvoker {
    async fn invoke(
        &self,
        thread_id: Option<&str>,
        message: &str,
    ) -> Result<InvocationOutput, String> {
        Ok(InvocationOutput {
            assistant_message: format!("echo: {message}"),
            thread_id: thread_id.unwrap_or("t-new").to_string(),
        })
    }
}

fn make_app() -> (axum::Router, JobStore) {
    let store = JobStore::new(Duration::from_secs(3600));
    let invoker: Arc<dyn AgentInvoker> = Arc::new(EchoInvoker);
    let app = router(store.clone(), invoker, Duration::from_secs(5));
    (app, store)
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn post_run_with_valid_body_returns_202_with_job_id() {
    let (app, _store) = make_app();
    let req = Request::builder()
        .method("POST")
        .uri("/run")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({ "payload": { "message": "hi" } }).to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let body = body_json(resp).await;
    let id = body.get("job_id").and_then(|v| v.as_str()).unwrap();
    assert_eq!(id.len(), 36);
}

#[tokio::test]
async fn post_run_missing_payload_returns_400() {
    let (app, _store) = make_app();
    let req = Request::builder()
        .method("POST")
        .uri("/run")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn post_run_empty_message_returns_400() {
    let (app, _store) = make_app();
    let req = Request::builder()
        .method("POST")
        .uri("/run")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({ "payload": { "message": "" } }).to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert!(body.get("error").is_some());
}

#[tokio::test]
async fn post_run_malformed_json_returns_400() {
    let (app, _store) = make_app();
    let req = Request::builder()
        .method("POST")
        .uri("/run")
        .header("content-type", "application/json")
        .body(Body::from("{not json"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
```

- [ ] **Step 5.2: Run tests — expect compile failure**

```bash
cargo test --manifest-path Cargo.toml --lib openhuman::agentbox::http 2>&1 | tail -10
```

Expected: `router` not defined.

- [ ] **Step 5.3: Implement HTTP layer with `/run` only**

Create `src/openhuman/agentbox/http.rs`:

```rust
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

use super::invoker::SharedInvoker;
use super::ops::submit_run;
use super::store::JobStore;
use super::types::{RunRequest, RunResponse};

#[derive(Clone)]
struct HttpState {
    store: JobStore,
    invoker: SharedInvoker,
    job_timeout: Duration,
}

/// Build the AgentBox sub-router.
///
/// `job_timeout` caps how long any single agent invocation may run before the
/// worker forces it to `failed`.
pub fn router(store: JobStore, invoker: SharedInvoker, job_timeout: Duration) -> Router {
    Router::new()
        .route("/run", post(post_run))
        .route("/jobs/{job_id}", get(get_job))
        .with_state(Arc::new(HttpState {
            store,
            invoker,
            job_timeout,
        }))
}

async fn post_run(
    State(state): State<Arc<HttpState>>,
    body: Result<Json<RunRequest>, axum::extract::rejection::JsonRejection>,
) -> impl IntoResponse {
    let Json(req) = match body {
        Ok(j) => j,
        Err(rej) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": rej.to_string() })),
            )
                .into_response();
        }
    };

    if req.payload.message.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "payload.message must be a non-empty string" })),
        )
            .into_response();
    }

    let id = submit_run(
        state.store.clone(),
        state.invoker.clone(),
        req.payload,
        state.job_timeout,
    );
    log::info!("[agentbox] /run accepted job_id={}", id);
    (StatusCode::ACCEPTED, Json(RunResponse { job_id: id })).into_response()
}

async fn get_job(
    State(_state): State<Arc<HttpState>>,
    axum::extract::Path(_job_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    // Implemented in Task 6.
    (StatusCode::NOT_IMPLEMENTED, Json(json!({ "error": "not yet" }))).into_response()
}
```

- [ ] **Step 5.4: Run tests — expect the 4 POST tests pass**

```bash
cargo test --manifest-path Cargo.toml --lib openhuman::agentbox::http 2>&1 | tail -15
```

Expected: `post_run_*` tests pass (4 of them).

- [ ] **Step 5.5: Commit**

```bash
git add src/openhuman/agentbox/http.rs src/openhuman/agentbox/http_tests.rs
git commit -m "feat(agentbox): POST /run handler returning 202 with job id (#3620)"
```

---

## Task 6: HTTP layer — `GET /jobs/{job_id}` (TDD)

**Files:**
- Modify: `src/openhuman/agentbox/http.rs`
- Modify: `src/openhuman/agentbox/http_tests.rs`

- [ ] **Step 6.1: Append failing tests for GET /jobs**

Append to `src/openhuman/agentbox/http_tests.rs`:

```rust
#[tokio::test]
async fn get_unknown_job_returns_404() {
    let (app, _store) = make_app();
    let req = Request::builder()
        .method("GET")
        .uri("/jobs/does-not-exist")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body = body_json(resp).await;
    assert_eq!(body.get("error").and_then(|v| v.as_str()), Some("job not found"));
}

#[tokio::test]
async fn run_then_poll_until_completed_returns_assistant_message() {
    let (app, _store) = make_app();

    // Submit
    let submit = Request::builder()
        .method("POST")
        .uri("/run")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({ "payload": { "message": "ping", "thread_id": "t-ext" } }).to_string(),
        ))
        .unwrap();
    let resp = app.clone().oneshot(submit).await.unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let id = body_json(resp).await["job_id"].as_str().unwrap().to_string();

    // Poll until completed (EchoInvoker is fast — bounded retries)
    let mut last = None;
    for _ in 0..50 {
        let poll = Request::builder()
            .method("GET")
            .uri(format!("/jobs/{id}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(poll).await.unwrap();
        let body = body_json(resp).await;
        if body["status"] == "completed" {
            last = Some(body);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let body = last.expect("job did not complete in time");
    assert_eq!(body["result"]["message"], "echo: ping");
    assert_eq!(body["result"]["thread_id"], "t-ext");
}
```

- [ ] **Step 6.2: Run tests — expect new tests fail**

```bash
cargo test --manifest-path Cargo.toml --lib openhuman::agentbox::http 2>&1 | tail -15
```

Expected: `get_unknown_job_returns_404` fails (returns 501 not 404), `run_then_poll_until_completed_returns_assistant_message` fails.

- [ ] **Step 6.3: Implement `get_job`**

Replace the stub `get_job` in `src/openhuman/agentbox/http.rs` with:

```rust
async fn get_job(
    State(state): State<Arc<HttpState>>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    match state.store.get(&job_id) {
        Some(view) => (StatusCode::OK, Json(view)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "job not found" })),
        )
            .into_response(),
    }
}
```

- [ ] **Step 6.4: Run tests — expect all pass**

```bash
cargo test --manifest-path Cargo.toml --lib openhuman::agentbox::http 2>&1 | tail -15
```

Expected: 6 passed.

- [ ] **Step 6.5: Commit**

```bash
git add src/openhuman/agentbox/http.rs src/openhuman/agentbox/http_tests.rs
git commit -m "feat(agentbox): GET /jobs/{job_id} handler (#3620)"
```

---

## Task 7: Wire AgentBox routes into the core HTTP router + auth bypass

**Files:**
- Modify: `src/core/auth.rs`
- Modify: `src/core/jsonrpc.rs`

- [ ] **Step 7.1: Add path prefixes to the public path list**

Open `src/core/auth.rs`. Find `PUBLIC_PATHS` (around line 78). The current list has exact paths only; we need prefix matching for `/jobs/{id}`. Inspect `is_public_path` or wherever `PUBLIC_PATHS` is consulted to see how matching works.

```bash
grep -n "PUBLIC_PATHS\|is_public_path\|is_external_inference_path" src/core/auth.rs | head -20
```

If matching is exact-only, add prefix support specifically for AgentBox paths. Otherwise add `"/run"` directly. Apply the right pattern based on what you find — example using exact match for `/run` and a small prefix helper for `/jobs/`:

```rust
const PUBLIC_PATHS: &[&str] = &[
    "/",
    "/health",
    "/auth",
    "/auth/telegram",
    "/schema",
    "/events",
    "/ws/dictation",
    "/run",
];

const PUBLIC_PATH_PREFIXES: &[&str] = &["/jobs/"];

// Wherever the existing matcher lives:
fn is_public_path(path: &str) -> bool {
    PUBLIC_PATHS.contains(&path)
        || PUBLIC_PATH_PREFIXES.iter().any(|p| path.starts_with(p))
}
```

If a matcher already exists, integrate the prefix check there. Do NOT introduce a parallel matcher.

- [ ] **Step 7.2: Add a test for the auth bypass**

Find existing `#[test]` cases in `src/core/auth.rs` (search for `fn is_external_inference_path` tests or similar). Add:

```rust
#[test]
fn agentbox_run_and_jobs_paths_are_public() {
    assert!(is_public_path("/run"));
    assert!(is_public_path("/jobs/abc-123"));
    assert!(is_public_path("/jobs/00000000-0000-0000-0000-000000000000"));
    // sanity: still protect the executable surface
    assert!(!is_public_path("/rpc"));
    assert!(!is_public_path("/v1/chat/completions"));
}
```

Use the actual function name surfaced by Step 7.1. If the function is `pub(crate)`, the test goes in the same file.

- [ ] **Step 7.3: Mount the AgentBox router in `build_core_http_router`**

Open `src/core/jsonrpc.rs`. Find `build_core_http_router` (around line 866).

The current signature is `pub fn build_core_http_router(socketio_enabled: bool) -> Router`. We need to pass in the `JobStore` and `SharedInvoker`, OR construct them inside the function when AgentBox mode is on.

Choose the latter to keep the call sites simple — only the embedded server constructs the router so there is one creation point:

```rust
use std::time::Duration;

pub fn build_core_http_router(socketio_enabled: bool) -> Router {
    let mut router = Router::new()
        .route("/", get(root_handler))
        .route("/health", get(health_handler))
        // ... existing routes unchanged ...
        ;

    // Mount AgentBox marketplace routes when explicitly enabled.
    if std::env::var("OPENHUMAN_AGENTBOX_MODE").as_deref() == Ok("1") {
        let store = crate::openhuman::agentbox::JobStore::new(Duration::from_secs(3600));
        let invoker: std::sync::Arc<dyn crate::openhuman::agentbox::invoker::AgentInvoker> =
            std::sync::Arc::new(crate::openhuman::agentbox::invoker::CoreAgentInvoker);
        let job_timeout = std::env::var("OPENHUMAN_AGENTBOX_JOB_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(600));

        // Spawn sweep loop — bounds memory under sustained traffic.
        let sweep_store = store.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            loop {
                tick.tick().await;
                let evicted = sweep_store.sweep_now();
                if evicted > 0 {
                    log::info!("[agentbox] sweep evicted {} terminal jobs", evicted);
                }
            }
        });

        log::info!(
            "[agentbox] enabled; public routes: POST /run, GET /jobs/{{id}}, GET /health"
        );
        router = router.merge(crate::openhuman::agentbox::agentbox_router(
            store, invoker, job_timeout,
        ));
    }

    // ... existing `.fallback`, `.layer(...)` chain stays ...
    router
}
```

Note: this changes the `let router = Router::new()...` binding from immutable to mutable (`let mut router`). Be careful to preserve the **exact** order of `.layer(...)` / `.fallback(...)` / `.with_state(...)` calls that follow the original chain — those still apply once to the final router. Verify by diffing against the pre-change file.

- [ ] **Step 7.4: Verify compile**

```bash
cargo check --manifest-path Cargo.toml --bin openhuman-core 2>&1 | tail -10
```

Expected: OK.

- [ ] **Step 7.5: Run auth tests**

```bash
cargo test --manifest-path Cargo.toml --lib core::auth 2>&1 | tail -10
```

Expected: existing + new test pass.

- [ ] **Step 7.6: Commit**

```bash
git add src/core/auth.rs src/core/jsonrpc.rs
git commit -m "feat(agentbox): mount /run + /jobs routes behind OPENHUMAN_AGENTBOX_MODE flag (#3620)"
```

---

## Task 8: Disabled-mode integration test (verify zero-impact on desktop)

**Files:**
- Create: `src/openhuman/agentbox/disabled_mode_tests.rs`
- Modify: `src/openhuman/agentbox/mod.rs`

- [ ] **Step 8.1: Add the test module**

Append to `src/openhuman/agentbox/mod.rs`:

```rust
#[cfg(test)]
mod disabled_mode_tests;
```

- [ ] **Step 8.2: Write the test**

Create `src/openhuman/agentbox/disabled_mode_tests.rs`:

```rust
//! Verifies that with `OPENHUMAN_AGENTBOX_MODE` unset (the desktop default),
//! the core HTTP router does NOT expose `/run` or `/jobs/{id}`.
//!
//! Uses serial_test pattern via an in-process env-var swap. If the codebase
//! has a project-standard serial_test crate, use it; otherwise the test is
//! safe because it sets and unsets the var within the same `#[test]` body.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn run_route_absent_when_mode_off() {
    // Ensure flag is OFF for this test.
    std::env::remove_var("OPENHUMAN_AGENTBOX_MODE");

    let router = crate::core::jsonrpc::build_core_http_router(false);
    let req = Request::builder()
        .method("POST")
        .uri("/run")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"payload":{"message":"x"}}"#))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    // The router's fallback returns 404 for unmounted routes.
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
```

- [ ] **Step 8.3: Run the test**

```bash
cargo test --manifest-path Cargo.toml --lib openhuman::agentbox::disabled_mode 2>&1 | tail -10
```

Expected: pass.

If the test races with other tests that set the env var, mark with `#[ignore]` and document why, OR wrap with whatever serial-test pattern the repo already uses (`grep -rn "serial_test\|#\[serial\]" src/`).

- [ ] **Step 8.4: Commit**

```bash
git add src/openhuman/agentbox/disabled_mode_tests.rs src/openhuman/agentbox/mod.rs
git commit -m "test(agentbox): verify routes are absent with mode flag off (#3620)"
```

---

## Task 9: Wire `CoreAgentInvoker` to the real agent runtime

**Files:**
- Modify: `src/openhuman/agentbox/invoker.rs`

This is the integration task. We bridge AgentBox `/run` to the existing web-channel chat path, which already drives the full agent runtime (skills, tools, memory, approval gate).

- [ ] **Step 9.1: Survey the bridging surface**

The web channel exposes `channel_web_chat` (`src/openhuman/channels/providers/web/ops.rs:544`) — it returns a `request_id` immediately and runs the agent in the background via an internal queue. The final assistant turn is published over the event bus.

Run:

```bash
grep -rn "AssistantTurnCompleted\|TurnCompleted\|ChatCompleted\|publish_global.*agent\|publish_global.*thread" src/openhuman/ --include="*.rs" | head -30
```

Identify the event variant that carries the final assistant text (likely under `DomainEvent::Agent` or `DomainEvent::Thread`). Note the variant name and the field that holds the response text.

If no single event carries the final text, fall back to the thread-store approach:

```bash
grep -rn "fn get_latest_assistant\|last_assistant_message\|read_thread" src/openhuman/threads/ --include="*.rs" | head -10
```

Pick the cleanest of: (a) subscribe to the completion event, (b) poll the thread store after `channel_web_chat` returns, (c) both.

- [ ] **Step 9.2: Implement the bridge**

Replace the stub `CoreAgentInvoker` impl in `src/openhuman/agentbox/invoker.rs`. The shape (the specifics fill in from Step 9.1):

```rust
use crate::core::event_bus::{subscribe_global, DomainEvent};

#[async_trait]
impl AgentInvoker for CoreAgentInvoker {
    async fn invoke(
        &self,
        thread_id: Option<&str>,
        message: &str,
    ) -> Result<InvocationOutput, String> {
        // 1. Resolve or create a thread.
        let thread_id_resolved = match thread_id {
            Some(id) => id.to_string(),
            None => {
                // Use the existing thread-creation op. Look up:
                //   grep -rn "fn create_thread\|new_thread" src/openhuman/threads/ops.rs
                // and call it. Fail with a string error on any failure.
                crate::openhuman::threads::ops::create_blank_thread()
                    .await
                    .map_err(|e| format!("create thread: {e}"))?
            }
        };

        // 2. Subscribe to completion events BEFORE submitting so we cannot
        //    race past the publish.
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
        let thread_match = thread_id_resolved.clone();
        let subscription = subscribe_global(move |event: &DomainEvent| {
            // Match against the specific variant identified in Step 9.1.
            // Pseudocode — adapt to actual variant name and field:
            // if let DomainEvent::AgentTurnCompleted { thread_id: t, response, .. } = event {
            //     if t == &thread_match {
            //         let _ = tx.send(Ok(response.clone()));
            //     }
            // }
            let _ = (event, &thread_match);
        });

        // 3. Submit the chat via the existing entrypoint.
        let metadata = crate::openhuman::channels::providers::web::types::ChatRequestMetadata::agentbox();
        crate::openhuman::channels::providers::web::ops::channel_web_chat(
            "agentbox",
            &thread_id_resolved,
            message,
            None, // model_override — provider already configured via GMI bridge
            None, // temperature
            None, // profile_id
            None, // locale
            None, // queue_mode
            metadata,
        )
        .await
        .map_err(|e| format!("submit chat: {e}"))?;

        // 4. Wait for the completion event.
        let response = rx
            .await
            .map_err(|_| "completion channel dropped before response".to_string())??;

        drop(subscription); // explicit unsubscribe

        Ok(InvocationOutput {
            assistant_message: response,
            thread_id: thread_id_resolved,
        })
    }
}
```

Two things you must finalize from real inspection:

1. **The event variant**: the closure body in Step 9.2 above is pseudocode. Use the actual variant identified in Step 9.1.
2. **`ChatRequestMetadata::agentbox()`**: add a new constructor on `ChatRequestMetadata` that tags the origin so the approval gate treats it as background-eligible. Look at existing constructors:

   ```bash
   grep -n "impl ChatRequestMetadata\|fn .*ChatRequestMetadata" src/openhuman/channels/providers/web/types.rs
   ```

   Add:

   ```rust
   pub fn agentbox() -> Self {
       // Match the field set used by existing constructors. Mark origin as
       // background/external to bypass interactive approval parking.
       Self {
           // ... copy default-ish values from the existing simplest constructor ...
       }
   }
   ```

- [ ] **Step 9.3: Compile and verify**

```bash
cargo check --manifest-path Cargo.toml --bin openhuman-core 2>&1 | tail -15
```

Expected: OK. If any types or fields surface unexpectedly, narrow the implementation to use whatever exists.

- [ ] **Step 9.4: Add an integration test of the bridge against a stub provider**

This is harder because we need the agent runtime to run. Skip a unit test here and let Task 12's `tests/agentbox_e2e.rs` cover it.

- [ ] **Step 9.5: Commit**

```bash
git add src/openhuman/agentbox/invoker.rs src/openhuman/channels/providers/web/types.rs
git commit -m "feat(agentbox): bridge AgentInvoker to live agent runtime via web channel (#3620)"
```

---

## Task 10: GMI MaaS env-var bridge (TDD)

**Files:**
- Create: `src/openhuman/agentbox/env.rs`
- Create: `src/openhuman/agentbox/env_tests.rs`
- Modify: `src/core/jsonrpc.rs` (call the bridge at startup)

- [ ] **Step 10.1: Write failing tests**

Create `src/openhuman/agentbox/env_tests.rs`:

```rust
use super::env::{collect_gmi_config, GmiConfig};

#[test]
fn collect_returns_some_when_all_three_vars_present() {
    let cfg = collect_gmi_config(|k| match k {
        "GMI_MAAS_BASE_URL" => Some("https://api.gmi-serving.com".into()),
        "GMI_MAAS_API_KEY" => Some("sk-test".into()),
        "GMI_MODELS" => Some("deepseek-ai/DeepSeek-V4-Pro".into()),
        _ => None,
    });
    assert_eq!(
        cfg,
        Ok(GmiConfig {
            base_url: "https://api.gmi-serving.com".into(),
            api_key: "sk-test".into(),
            model: "deepseek-ai/DeepSeek-V4-Pro".into(),
        })
    );
}

#[test]
fn collect_reports_each_missing_var() {
    let cfg = collect_gmi_config(|k| match k {
        "GMI_MAAS_BASE_URL" => Some("u".into()),
        _ => None,
    });
    let err = cfg.unwrap_err();
    assert!(err.contains("GMI_MAAS_API_KEY"), "missing api key reported");
    assert!(err.contains("GMI_MODELS"), "missing model reported");
    assert!(
        !err.contains("GMI_MAAS_BASE_URL"),
        "present var not reported missing"
    );
}

#[test]
fn collect_treats_blank_string_as_missing() {
    let cfg = collect_gmi_config(|k| match k {
        "GMI_MAAS_BASE_URL" => Some("".into()),
        "GMI_MAAS_API_KEY" => Some("sk".into()),
        "GMI_MODELS" => Some("m".into()),
        _ => None,
    });
    assert!(cfg.is_err());
}
```

- [ ] **Step 10.2: Run tests — expect compile failure**

```bash
cargo test --manifest-path Cargo.toml --lib openhuman::agentbox::env 2>&1 | tail -10
```

- [ ] **Step 10.3: Implement the env module**

Create `src/openhuman/agentbox/env.rs`:

```rust
//! Reads `GMI_MAAS_*` env vars injected by AgentBox at runtime and registers
//! an OpenAI-compatible cloud provider so the agent runtime can call the
//! marketplace's MaaS inference endpoint.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmiConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

/// Collect GMI config from a getter (real env or test fake).
///
/// Returns `Ok(_)` only when all three vars are present and non-blank. The
/// error string lists every missing var so the operator can fix all at once.
pub fn collect_gmi_config<F>(get: F) -> Result<GmiConfig, String>
where
    F: Fn(&str) -> Option<String>,
{
    let base_url = nonblank(&get, "GMI_MAAS_BASE_URL");
    let api_key = nonblank(&get, "GMI_MAAS_API_KEY");
    let model = nonblank(&get, "GMI_MODELS");

    let mut missing = Vec::new();
    if base_url.is_none() {
        missing.push("GMI_MAAS_BASE_URL");
    }
    if api_key.is_none() {
        missing.push("GMI_MAAS_API_KEY");
    }
    if model.is_none() {
        missing.push("GMI_MODELS");
    }
    if !missing.is_empty() {
        return Err(format!("missing/blank: {}", missing.join(", ")));
    }
    Ok(GmiConfig {
        base_url: base_url.unwrap(),
        api_key: api_key.unwrap(),
        model: model.unwrap(),
    })
}

fn nonblank<F: Fn(&str) -> Option<String>>(get: &F, key: &str) -> Option<String> {
    get(key).filter(|v| !v.trim().is_empty())
}

/// Read env and register the GMI MaaS provider on startup if available.
///
/// No-op (with a warning log) if any required var is missing — the core still
/// boots in degraded mode, useful for local testing of `/run` without GMI.
///
/// **Never logs the API key.**
pub fn register_gmi_provider_if_present() {
    let cfg = match collect_gmi_config(|k| std::env::var(k).ok()) {
        Ok(cfg) => cfg,
        Err(reason) => {
            log::warn!(
                "[agentbox::gmi] not registering GMI MaaS provider: {}",
                reason
            );
            return;
        }
    };

    log::info!(
        "[agentbox::gmi] registering provider base_url={} model={}",
        cfg.base_url,
        cfg.model
    );

    // Delegate the actual provider registration to the existing
    // compatible_provider catalog. The function name varies by codebase —
    // look up:
    //   grep -rn "register_cloud_provider\|add_compatible_provider" src/openhuman/inference/
    // and call the appropriate registrar with cfg.base_url, cfg.api_key,
    // cfg.model as the default. Use provider name "gmi-maas".
    register_gmi_with_inference_catalog(&cfg);
}

fn register_gmi_with_inference_catalog(_cfg: &GmiConfig) {
    // Real implementation lands here once Step 10.5 finalizes the registrar
    // function name. Wire up the OpenAI-compatible provider builder using
    // base_url + api_key + model.
}
```

- [ ] **Step 10.4: Run tests — expect pass**

```bash
cargo test --manifest-path Cargo.toml --lib openhuman::agentbox::env 2>&1 | tail -10
```

Expected: 3 passed.

- [ ] **Step 10.5: Wire the actual provider registration**

Discover the registrar:

```bash
grep -rn "OpenAiCompatibleProvider::new\|register.*provider\|fn register_" src/openhuman/inference/ --include="*.rs" | head -20
```

Replace the stub body of `register_gmi_with_inference_catalog` with a call to the real registrar passing `cfg.base_url`, `cfg.api_key`, `cfg.model`. If no registrar exists at runtime (the codebase may use TOML config exclusively for provider catalog), the next-best option is to update the in-memory `Config` via the same path `config.update_*` RPCs use, marking GMI as the active provider for the agent runtime. The implementer has discretion here based on what they find — but the contract is fixed: after this call, agent invocations must reach the GMI base URL.

Add at least one log line confirming registration succeeded (no key logged).

- [ ] **Step 10.6: Call the bridge from startup**

Open `src/core/jsonrpc.rs::run_server_inner`. After the keyring init (around line 1462) and before the agent runtime starts accepting work, add:

```rust
// AgentBox GMI MaaS provider bridge — no-op when env vars absent.
crate::openhuman::agentbox::register_gmi_provider_if_present();
```

- [ ] **Step 10.7: Verify compile**

```bash
cargo check --manifest-path Cargo.toml --bin openhuman-core 2>&1 | tail -10
```

- [ ] **Step 10.8: Commit**

```bash
git add src/openhuman/agentbox/env.rs src/openhuman/agentbox/env_tests.rs src/core/jsonrpc.rs
git commit -m "feat(agentbox): GMI MaaS env-var bridge to inference provider catalog (#3620)"
```

---

## Task 11: Dockerfile + `.env.example` defaults

**Files:**
- Modify: `Dockerfile`
- Modify: `.env.example`

- [ ] **Step 11.1: Add defaults to the Dockerfile runtime stage**

In `Dockerfile`, find the existing `ENV OPENHUMAN_WORKSPACE=…` block (around line 110-115). Add immediately after:

```dockerfile
# AgentBox marketplace mode — off by default for desktop builds. The
# AgentBox console flips this on per deployment, along with GMI_MAAS_*.
ENV OPENHUMAN_AGENTBOX_MODE=0
```

- [ ] **Step 11.2: Document env vars in `.env.example`**

Append to `.env.example`:

```bash
# ---------------------------------------------------------------------------
# AgentBox marketplace (GMI Cloud) — opt-in container surface.
# Set OPENHUMAN_AGENTBOX_MODE=1 to expose POST /run and GET /jobs/{job_id}.
# The remaining GMI_MAAS_* and GMI_MODELS variables are injected by the
# AgentBox console at deploy time; for local testing of the /run path set
# them yourself (they MUST be non-blank).
# ---------------------------------------------------------------------------
# OPENHUMAN_AGENTBOX_MODE=0
# OPENHUMAN_AGENTBOX_JOB_TIMEOUT_SECS=600
# GMI_MAAS_BASE_URL=https://api.gmi-serving.com
# GMI_MAAS_API_KEY=
# GMI_MODELS=deepseek-ai/DeepSeek-V4-Pro
```

- [ ] **Step 11.3: Commit**

```bash
git add Dockerfile .env.example
git commit -m "chore(agentbox): document mode + GMI vars in Dockerfile and .env.example (#3620)"
```

---

## Task 12: End-to-end test against the built binary

**Files:**
- Create: `tests/agentbox_e2e.rs`

- [ ] **Step 12.1: Look at existing shell-launched binary tests**

```bash
ls tests/ 2>/dev/null
cat scripts/test-rust-with-mock.sh 2>/dev/null | head -40
```

Identify the pattern: how an existing integration test boots `openhuman-core`, sets workspace, polls `/health`, then exercises endpoints.

- [ ] **Step 12.2: Write the e2e test**

Create `tests/agentbox_e2e.rs`:

```rust
//! End-to-end: boot openhuman-core with AgentBox mode + a stubbed inference
//! provider, submit a /run, poll until completed.
//!
//! Mirrors the pattern in `tests/json_rpc_e2e.rs` (binary + mock provider).

#![cfg(not(target_os = "windows"))] // POSIX-only fixture script

use std::process::{Child, Command, Stdio};
use std::time::Duration;

// Reuse helpers from json_rpc_e2e.rs — if those aren't exposed across test
// crates, copy the minimum (port allocation, workspace tempdir, child reaping).

fn start_core_with_agentbox(port: u16, workspace: &std::path::Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_openhuman-core"))
        .arg("serve")
        .env("OPENHUMAN_AGENTBOX_MODE", "1")
        .env("OPENHUMAN_CORE_PORT", port.to_string())
        .env("OPENHUMAN_WORKSPACE", workspace)
        // Point at the mock server started by test-rust-with-mock.sh:
        .env("GMI_MAAS_BASE_URL", std::env::var("MOCK_API_URL").unwrap_or("http://127.0.0.1:4010".into()))
        .env("GMI_MAAS_API_KEY", "test-key")
        .env("GMI_MODELS", "stub-model")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn openhuman-core")
}

async fn wait_health(port: u16) {
    let client = reqwest::Client::new();
    for _ in 0..50 {
        if let Ok(r) = client
            .get(format!("http://127.0.0.1:{port}/health"))
            .send()
            .await
        {
            if r.status().is_success() {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("core did not become healthy");
}

#[tokio::test]
async fn agentbox_run_then_poll_completes() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let port = 17788_u16; // pick a free-ish port; bump if it collides in CI
    let mut child = start_core_with_agentbox(port, workspace.path());
    let _guard = scopeguard::guard((), |_| {
        let _ = child.kill();
        let _ = child.wait();
    });

    wait_health(port).await;

    let client = reqwest::Client::new();
    // Submit
    let resp = client
        .post(format!("http://127.0.0.1:{port}/run"))
        .json(&serde_json::json!({ "payload": { "message": "hello" } }))
        .send()
        .await
        .expect("submit /run");
    assert_eq!(resp.status(), 202);
    let job_id: String = resp
        .json::<serde_json::Value>()
        .await
        .unwrap()
        .get("job_id")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();

    // Poll
    let mut final_status: Option<String> = None;
    for _ in 0..100 {
        let r = client
            .get(format!("http://127.0.0.1:{port}/jobs/{job_id}"))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = r.json().await.unwrap();
        let status = body["status"].as_str().unwrap().to_string();
        if status == "completed" || status == "failed" {
            final_status = Some(status);
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(
        final_status.as_deref(),
        Some("completed"),
        "job should complete against mock provider"
    );
}
```

Note: `tempfile`, `scopeguard`, `reqwest` may or may not be `dev-dependencies` already — check `[dev-dependencies]` in `Cargo.toml` and add what's missing. If `reqwest` isn't a dev-dep, use a smaller HTTP client already in use (search `grep -n "reqwest\|ureq" Cargo.toml`).

- [ ] **Step 12.3: Add any missing dev-dependencies**

If needed, append to `Cargo.toml` `[dev-dependencies]`:

```toml
scopeguard = "1"
tempfile = "3"
# Only add reqwest if not already present.
```

- [ ] **Step 12.4: Run the e2e test (with mock)**

```bash
bash scripts/test-rust-with-mock.sh --test agentbox_e2e 2>&1 | tail -30
```

Expected: pass. If the test infrastructure expects a specific port range or fixture, follow the existing `json_rpc_e2e.rs` setup exactly.

- [ ] **Step 12.5: Commit**

```bash
git add tests/agentbox_e2e.rs Cargo.toml
git commit -m "test(agentbox): end-to-end /run + /jobs against built binary (#3620)"
```

---

## Task 13: Deployment runbook

**Files:**
- Create: `gitbooks/developing/agentbox-deployment.md`

- [ ] **Step 13.1: Write the runbook**

Create `gitbooks/developing/agentbox-deployment.md`:

```markdown
# AgentBox Marketplace Deployment

OpenHuman ships as a containerized agent on GMI Cloud's
[AgentBox marketplace](https://docs.gmicloud.ai/agentbox-marketplace/overview).
This page is the operator runbook for new deployments and version bumps.

## Container contract

When `OPENHUMAN_AGENTBOX_MODE=1`, the core HTTP server exposes:

- `POST /run` — accept work, return `202 { "job_id": "<uuid>" }`. Body shape:
  `{ "payload": { "message": "<string>", "thread_id": "<optional string>" } }`.
- `GET /jobs/{job_id}` — return `{ "status": "pending|running|completed|failed", "result": ..., "error": ... }`.
- `GET /health` — liveness.

Both `/run` and `/jobs/*` are unauthenticated at the container boundary —
AgentBox's edge handles auth before traffic reaches us.

## The 4-step register wizard

In the AgentBox console:

1. **Basic Info** — name `OpenHuman`, description, listing identity.
2. **Infrastructure** — Docker image source (push tagged builds to your
   chosen registry, see "Image push" below), compute tier, region. Enable the
   "GMI MaaS" toggle so the platform injects `GMI_MAAS_BASE_URL` and
   `GMI_MAAS_API_KEY` at runtime.
3. **Env Variables** — set:
   - `OPENHUMAN_AGENTBOX_MODE=1`
   - (optional) `OPENHUMAN_AGENTBOX_JOB_TIMEOUT_SECS` (default 600)
   - `GMI_MODELS` to the marketplace-approved model id (e.g.
     `deepseek-ai/DeepSeek-V4-Pro`).
   - `OPENHUMAN_WORKSPACE` to a writable container path (e.g. `/home/openhuman/.openhuman`).
   - `RUST_LOG=info` (or `debug` while shaking out the first deploy).
4. **Review & Register** — confirm and test from the console panel.

> ⚠️ The platform API key is shown ONCE on the registration confirmation
> screen. Save it to your secrets manager immediately. It is NOT recoverable
> from the console after that.

## Image push

Build and push from `main` using the existing `Dockerfile`:

```bash
docker build -t <registry>/openhuman-core:<tag> .
docker push <registry>/openhuman-core:<tag>
```

First deploy takes 10–25 minutes to reach `running`; later deploys are faster.

## Long-running requests

AgentBox treats requests >2 min as long-running. OpenHuman handles this with
**polling** per AgentBox's documented pattern — the agent runtime is invoked
inside the worker task, capped by `OPENHUMAN_AGENTBOX_JOB_TIMEOUT_SECS`
(default 10 minutes). No streaming.

Polling clients should:

1. `POST /run` and capture `job_id`.
2. `GET /jobs/{job_id}` every 1–3 seconds.
3. Stop when `status` is `completed` or `failed`.
4. Note: terminal jobs are retained for 1 hour after completion, then
   garbage-collected. Long pauses between poll and read may return `404`.

## Local smoke test

```bash
OPENHUMAN_AGENTBOX_MODE=1 \
GMI_MAAS_BASE_URL=https://api.gmi-serving.com \
GMI_MAAS_API_KEY=sk-... \
GMI_MODELS=deepseek-ai/DeepSeek-V4-Pro \
./target/debug/openhuman-core serve &

curl -X POST http://127.0.0.1:7788/run \
  -H 'content-type: application/json' \
  -d '{"payload":{"message":"hello"}}'

# Then poll the returned job_id:
curl http://127.0.0.1:7788/jobs/<job_id>
```

## Troubleshooting

- `404 job not found` after a successful submit — retention window (1h) has
  elapsed, or the container restarted (in-memory store is not durable in v1).
- `status: "failed"`, `error: "agentbox: agent runtime bridge not wired"` —
  this is the stub error from before Task 9; rebuild against a current
  `main`.
- `status: "failed"`, `error: "job timeout after Ns"` — the agent invocation
  exceeded `OPENHUMAN_AGENTBOX_JOB_TIMEOUT_SECS`. Bump the env var on the
  next deploy.
- `register_gmi_provider_if_present: missing/blank: GMI_MAAS_API_KEY` —
  the platform did not inject the key. Re-check the wizard's "MaaS
  integration toggle" in Step 2.
```

- [ ] **Step 13.2: Commit**

```bash
git add gitbooks/developing/agentbox-deployment.md
git commit -m "docs(agentbox): operator runbook for marketplace deployment (#3620)"
```

---

## Task 14: Final verification

- [ ] **Step 14.1: Run the full Rust test suite**

```bash
pnpm test:rust 2>&1 | tail -40
```

Expected: all tests pass, including new agentbox tests.

- [ ] **Step 14.2: Run `cargo fmt --check` and `cargo clippy`**

```bash
cargo fmt --manifest-path Cargo.toml -- --check
cargo clippy --manifest-path Cargo.toml --bin openhuman-core -- -D warnings 2>&1 | tail -20
```

Fix any formatter or lint findings inline.

- [ ] **Step 14.3: Verify zero impact on desktop build**

```bash
pnpm rust:check
```

Expected: OK. No new warnings tied to `agentbox` when `OPENHUMAN_AGENTBOX_MODE` is unset.

- [ ] **Step 14.4: Smoke-test the container locally (optional but recommended)**

```bash
docker build -t openhuman-core:agentbox-dev .
docker run -p 7788:7788 \
  -e OPENHUMAN_AGENTBOX_MODE=1 \
  -e GMI_MAAS_BASE_URL=https://api.gmi-serving.com \
  -e GMI_MAAS_API_KEY=$YOUR_TEST_KEY \
  -e GMI_MODELS=deepseek-ai/DeepSeek-V4-Pro \
  openhuman-core:agentbox-dev
```

In another terminal:

```bash
curl -X POST http://127.0.0.1:7788/run \
  -H 'content-type: application/json' \
  -d '{"payload":{"message":"hello"}}'
# capture job_id, then:
curl http://127.0.0.1:7788/jobs/<job_id>
```

Expected: `202` then eventually `{ "status": "completed", "result": ... }`.

- [ ] **Step 14.5: Open the PR**

Push the branch and open a PR against `tinyhumansai:main` from the fork:

```bash
git push aniketh feat/3620-agentbox-marketplace-spec
gh pr create --repo tinyhumansai/openhuman \
  --head CodeGhost21:feat/3620-agentbox-marketplace-spec \
  --title "feat(agentbox): GMI Cloud AgentBox marketplace adapter (#3620)" \
  --body-file - <<'EOF'
## Summary
- New `src/openhuman/agentbox/` domain exposing `POST /run` and `GET /jobs/{job_id}` per the AgentBox marketplace contract, behind `OPENHUMAN_AGENTBOX_MODE=1`.
- In-memory job store (1h retention), tokio-spawned worker per request, configurable timeout.
- Bridge `GMI_MAAS_*` env vars to the existing OpenAI-compatible cloud provider so the agent runtime calls GMI's MaaS at runtime.
- Zero impact on the desktop build — flag is off by default.

## Test plan
- [ ] `pnpm test:rust` passes (unit + http + e2e).
- [ ] `cargo fmt --check` clean.
- [ ] `pnpm rust:check` clean.
- [ ] Local Docker smoke: `POST /run` → poll → `completed`.
- [ ] Operator runbook reviewed: `gitbooks/developing/agentbox-deployment.md`.

## Out of scope (operator tasks per issue #3620)
- Marketplace console registration.
- Image push to a registry of record.
- Storing the platform API key in a secrets manager.
- End-to-end validation against the live AgentBox marketplace.

Closes #3620 (code side).
EOF
```

---

## Notes for the implementer

- **TDD is non-negotiable** for Tasks 2, 4, 5, 6, 10. Each must show a failing test first, then the implementation, then green.
- **Frequent commits.** One task = one (occasionally two) focused commits. Avoid bundling unrelated changes.
- **Never log secrets.** `GMI_MAAS_API_KEY` must not appear in any log line at any level, even truncated or hashed. CI greps for `GMI_MAAS_API_KEY` in test output; if it appears, the build fails.
- **Match existing patterns** for module shape (`AGENTS.md`'s "Canonical module shape" table) and HTTP-test style (the `tower::ServiceExt::oneshot` pattern in `src/openhuman/embeddings/openai_tests.rs` is a good reference).
- **Don't add abstractions beyond what's needed.** `AgentInvoker` is the one trait; resist the urge to add a `JobStore` trait or a per-provider strategy. YAGNI.
- **If Task 9's bridge surface differs from my pseudocode**, write what the codebase actually exposes — the spec's only contract is "full agent runtime, captures final assistant text."
