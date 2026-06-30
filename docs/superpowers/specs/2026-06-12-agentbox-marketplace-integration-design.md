# AgentBox Marketplace Integration

**Issue:** [tinyhumansai/openhuman#3620](https://github.com/tinyhumansai/openhuman/issues/3620)
**Status:** Draft — design approved, plan pending
**Date:** 2026-06-12

## Background

OpenHuman has been onboarded to the **AgentBox POC** on GMI Cloud — an agent marketplace where containerized agents are registered, deployed, and distributed. The platform invokes containers over HTTP with a polling-based long-running contract and injects an OpenAI-compatible LLM (GMI MaaS) at runtime.

This design covers the code-side work to make `openhuman-core` a valid AgentBox container. Operational tasks (console registration, image push, deployment, API-key handling in a secrets manager, end-to-end marketplace validation) are out of scope of this document and remain the operator's responsibility.

## AgentBox contract (as documented)

Verified from `https://docs.gmicloud.ai/agentbox-marketplace/`:

- `POST /run`
  - Request: `{ "payload": <agent-defined> }`
  - Response: `202 { "job_id": "<uuid>" }`
- `GET /jobs/{job_id}`
  - Response: `200 { "status": "pending" | "running" | "completed" | "failed", "result": {...}, "error": "..." }`
  - `404` for unknown ids.
- Pattern is **polling-only**. No SSE / WebSockets / chunked streaming.
- Platform-injected env vars at runtime:
  - `GMI_MAAS_BASE_URL` — OpenAI-compatible base URL.
  - `GMI_MAAS_API_KEY` — MaaS API key (must remain unset in image).
  - `GMI_MODELS` — model id to call.
- Container image source: any public/private Docker registry.
- Listening port, healthcheck endpoint, and `payload` shape are agent-defined.

## Goals

1. Stand up `POST /run` and `GET /jobs/{job_id}` in `openhuman-core`, conforming to the AgentBox contract.
2. Drive each `/run` invocation through the **full agent runtime** (skills, tools, memory) — the same path chat uses.
3. Plumb `GMI_MAAS_*` env vars into the inference provider catalog so the agent actually uses GMI MaaS at runtime.
4. Keep the change zero-impact on the desktop build (routes off by default).

## Non-goals

- Persistent job store. Jobs are kept in-memory; lost on container restart.
- Streaming responses. AgentBox's contract is polling-only.
- Marketplace registration / image push / API-key storage / e2e marketplace validation — operator tasks.
- A separate Dockerfile. The existing one is reused with new env defaults.
- Backwards-compatibility shims — `OPENHUMAN_AGENTBOX_MODE` is new; no migration needed.

## Architecture

A new domain `src/openhuman/agentbox/` with the canonical module shape:

| File           | Role                                                                                                              |
| -------------- | ----------------------------------------------------------------------------------------------------------------- |
| `mod.rs`       | Export-only: `pub mod` + `pub use`. No business logic.                                                            |
| `types.rs`     | `RunRequest`, `RunResponse`, `JobStatus`, `JobRecord`, `RunPayload`, `RunResult`.                                 |
| `store.rs`     | `JobStore` wrapping `DashMap<JobId, JobRecord>` behind `Arc`. Insert / get / update / sweep evicted terminal jobs.|
| `ops.rs`       | `submit_run`, `get_job`, `run_job` worker, sweep task.                                                            |
| `http.rs`      | Axum sub-router with `POST /run` and `GET /jobs/{job_id}`. Handler functions delegate to `ops.rs`.                |
| `env.rs`       | `register_gmi_provider_if_present()` — reads `GMI_MAAS_*` at startup and wires the OpenAI-compatible provider.    |
| `http_tests.rs`| Axum `TestServer` integration tests.                                                                              |

The router is mounted in `src/core/jsonrpc.rs::build_core_http_router` only when `OPENHUMAN_AGENTBOX_MODE=1`. Default is off, so desktop builds are unaffected.

### Why a new domain

Per `AGENTS.md`: "New functionality → dedicated subdirectory." AgentBox is a transport surface specific to GMI Cloud's marketplace contract. It is not a generic concern of the core HTTP server, so it does not belong in `src/core/`. It is not a domain-of-substance like `agent` or `threads`; it is an adapter layer that delegates inward.

### Mounting point

In `build_core_http_router`:

```rust
let router = Router::new()
    .route("/", get(root_handler))
    .route("/health", get(health_handler))
    // ... existing routes ...
    .nest("/v1", crate::openhuman::inference::http::router());

let router = if std::env::var("OPENHUMAN_AGENTBOX_MODE").as_deref() == Ok("1") {
    router.merge(crate::openhuman::agentbox::http::router(job_store.clone()))
} else {
    router
};
```

The `JobStore` is created once at startup in `run_server_embedded` and shared with both the AgentBox router and the sweep task.

## HTTP contract

### `POST /run`

Request body:
```json
{ "payload": { "message": "<string>", "thread_id": "<string?>" } }
```

`thread_id` is optional. When omitted, a new thread is created for the job. When supplied, the job runs in the existing thread (allowing multi-turn flows if the marketplace consumer threads state).

Successful response — `202 Accepted`:
```json
{ "job_id": "<uuid>" }
```

Error responses:
- `400 Bad Request` — `payload` missing, `message` missing or empty, or body not valid JSON. Body: `{ "error": "<reason>" }`.

### `GET /jobs/{job_id}`

Successful response — `200 OK`:
```json
{
  "status": "pending" | "running" | "completed" | "failed",
  "result": { "message": "<assistant reply>", "thread_id": "<string>" },
  "error": "<string>"
}
```

`result` is present only when `status == "completed"`. `error` is present only when `status == "failed"`. Field order matches AgentBox's documented response shape.

Error responses:
- `404 Not Found` — unknown or evicted job id. Body: `{ "error": "job not found" }`.

### Authentication

Both routes are **unauthenticated at the container boundary** — AgentBox's edge handles auth before reaching us. They are added to the public path list in `src/core/auth.rs` so `rpc_auth_middleware` skips them.

The existing `/rpc` route stays bearer-protected. In AgentBox deployments the container binds to `0.0.0.0`, so the bearer-protected routes are reachable on the network — that is acceptable because the bearer is generated per-launch and never leaves the container's runtime memory (no env-var exposure). For added defense, AgentBox mode logs a warning at startup reminding the operator that only `/run`, `/jobs/*`, and `/health` are intended to be public.

### Healthcheck

Reuse existing `GET /health`. No new contract.

### Routes summary

| Path             | Method | Auth     | Added by      |
| ---------------- | ------ | -------- | ------------- |
| `/health`        | GET    | none     | existing      |
| `/run`           | POST   | none     | this design   |
| `/jobs/{job_id}` | GET    | none     | this design   |
| `/rpc`           | POST   | bearer   | existing      |
| `/v1/*`          | *      | (existing) | existing    |

## Job execution

### Worker flow

`run_job(store, job_id, RunPayload { message, thread_id })`:

1. Update job status `pending → running` in the store.
2. Resolve thread: if `thread_id` supplied, fetch via `threads::ops`; otherwise create a new thread via existing thread-creation ops. Capture the thread id for the result.
3. Invoke the agent dispatcher with the user message. This goes through the **full agent runtime** — same entrypoint as chat: skills, tools, memory, prompt injection guard, the lot. Reuses the existing dispatcher's public submission API.
4. Await dispatcher completion. Capture the final assistant turn's text content.
5. Write `JobRecord { status: Completed, result: Some(RunResult { message: assistant_text, thread_id }), error: None }` to the store.
6. On any error (dispatcher error, thread-resolve error, timeout): write `JobRecord { status: Failed, result: None, error: Some(err_string) }`.

### Cancellation & timeout

- Hard cap configurable via `OPENHUMAN_AGENTBOX_JOB_TIMEOUT_SECS`. Default: `600` (10 minutes).
- Wrapped in `tokio::time::timeout`. On expiry, the future is dropped; the agent dispatcher's existing tokio cancellation semantics handle cleanup.
- On timeout: `status = "failed"`, `error = "job timeout after <N>s"`.

### Job retention

- Terminal jobs (`completed` / `failed`) are retained for **1 hour** after completion, then evicted by a background sweep.
- A `tokio::spawn` background task wakes every 60 seconds and removes jobs whose `terminal_at` is older than the retention window.
- This bounds memory under sustained traffic without forcing persistence in v1.
- If a polling client misses the retention window, they get `404 job not found` — acceptable per AgentBox's contract (no docs claim indefinite retention).

### Concurrency

- The `JobStore` is `DashMap`-backed and safe for concurrent insert/get/update.
- `submit_run` always spawns the worker via `tokio::spawn` and returns immediately. There is no semaphore in v1 — the underlying agent runtime already handles concurrency at its level.
- Future: if concurrent job pressure becomes a problem, add a configurable semaphore around `tokio::spawn`. Out of scope here.

## GMI MaaS provider bridge

### Startup

`env.rs::register_gmi_provider_if_present()` runs once during `run_server_embedded` initialization, before the agent runtime accepts work.

Reads:
- `GMI_MAAS_BASE_URL` — required.
- `GMI_MAAS_API_KEY` — required.
- `GMI_MODELS` — required, single model id (AgentBox docs example: `deepseek-ai/DeepSeek-V4-Pro`).

Behavior:
- All three present → register an OpenAI-compatible cloud provider named `"gmi-maas"` into the inference provider catalog using the existing `compatible_provider_impl`. The model from `GMI_MODELS` becomes its default. Mark `"gmi-maas"` as the active provider for agent runtime invocations.
- Any missing → log a single `warn!` line listing which vars are missing, skip registration, continue startup. The core still runs (useful for local testing of the `/run` route with a stubbed inference layer), but agent calls that need an LLM will fail until a provider is configured by other means.

### Logging rules

Per repo logging conventions: log the base URL and model id at `info!` level when registered; never log the API key (not even truncated). Use the stable `[agentbox]` prefix for AgentBox-side logs and `[agentbox::gmi]` for the provider bridge.

## Data shapes

```rust
// types.rs
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

#[derive(Debug, Clone, Serialize)]
pub struct RunResult {
    pub message: String,
    pub thread_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobView {
    pub status: JobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<RunResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct JobRecord {
    pub status: JobStatus,
    pub result: Option<RunResult>,
    pub error: Option<String>,
    pub created_at: std::time::Instant,
    pub terminal_at: Option<std::time::Instant>,
}
```

`JobView` is the serialization-only projection returned by `GET /jobs/{job_id}`. `JobRecord` is the internal state.

## Configuration surface

New env vars:

| Var                                   | Default      | Role                                              |
| ------------------------------------- | ------------ | ------------------------------------------------- |
| `OPENHUMAN_AGENTBOX_MODE`             | `0`          | `1` enables `/run` + `/jobs/*` routes.            |
| `OPENHUMAN_AGENTBOX_JOB_TIMEOUT_SECS` | `600`        | Hard cap per job invocation.                      |
| `GMI_MAAS_BASE_URL`                   | unset        | OpenAI-compatible base URL (AgentBox-injected).   |
| `GMI_MAAS_API_KEY`                    | unset        | MaaS API key (AgentBox-injected at runtime).      |
| `GMI_MODELS`                          | unset        | Model id (AgentBox-injected).                     |

All five are documented in `.env.example` with comments tying them back to the AgentBox console wizard.

## Testing

### Unit tests (inline `#[cfg(test)] mod tests`)

- `store.rs`
  - `insert` / `get` round-trip.
  - `mark_completed` and `mark_failed` update status and `terminal_at`.
  - `sweep` evicts terminal jobs older than the retention window; leaves running and recent terminal jobs untouched.
- `ops.rs`
  - `submit_run` returns a valid v4 uuid and a job in `Pending` status.
  - `run_job` happy path with a mocked dispatcher → `Completed` with the assistant message captured.
  - `run_job` dispatcher-error path → `Failed` with the error string captured.
  - `run_job` timeout path → `Failed` with `"job timeout..."`.
- `env.rs`
  - All three vars present → provider registered, `info!` logged with base URL and model (no key).
  - Any one missing → no provider registered, `warn!` logged listing missing vars.
  - `GMI_MAAS_API_KEY` value never appears in captured logs.

### HTTP integration tests (`http_tests.rs`)

Uses Axum's `TestServer`:

- `POST /run` with valid body → `202` and a uuid in `job_id`.
- `POST /run` with missing `message` → `400` and `{ "error": ... }` body.
- `POST /run` with malformed JSON → `400`.
- `GET /jobs/{job_id}` for unknown id → `404` and `{ "error": "job not found" }`.
- End-to-end with a fast mocked dispatcher: submit `/run`, poll `/jobs/{job_id}` until `completed`, assert result shape.
- Routes are not registered when `OPENHUMAN_AGENTBOX_MODE` is unset → `POST /run` returns `404`.

### Rust integration test in `tests/`

A `tests/agentbox_e2e.rs` against the real core binary started by `scripts/test-rust-with-mock.sh`:

- Boot core with `OPENHUMAN_AGENTBOX_MODE=1` and stubbed `GMI_MAAS_*`.
- Submit a `/run` with a canned message that the mock provider answers deterministically.
- Poll until `completed`, assert the assistant reply matches.

### Out of scope for tests

- E2E against the real AgentBox marketplace. That is the operator's manual acceptance step from the issue.

## Docker

The existing `Dockerfile` is unchanged structurally. Two additions:

1. Default `ENV OPENHUMAN_AGENTBOX_MODE=0` so desktop bundles built from the same Dockerfile do not flip on the public routes.
2. `EXPOSE 7788` stays — AgentBox does not require a specific port; the value is passed through during console registration.

Operator workflow at deploy time (set in the AgentBox console wizard's "Env Variables" step):
- `OPENHUMAN_AGENTBOX_MODE=1`
- `GMI_MAAS_BASE_URL` / `GMI_MAAS_API_KEY` / `GMI_MODELS` are injected automatically by AgentBox.
- Optional: `OPENHUMAN_AGENTBOX_JOB_TIMEOUT_SECS` override.

No image rebuild needed to switch modes.

## Documentation

Add `gitbooks/developing/agentbox-deployment.md` — a short runbook covering:

1. The 4-step AgentBox console wizard, mapped to our env vars.
2. The image tag/registry conventions to use for releases destined for the marketplace.
3. The polling-only contract recap (`/run` + `/jobs/*`) for anyone debugging from the AgentBox console's test panel.
4. Timeout & retention defaults and how to tune them.

No CLAUDE.md / AGENTS.md change — AgentBox is a transport surface, not a core convention.

## Risks & open questions

- **Job-store memory growth under burst traffic.** Mitigation: 1-hour retention + 60-second sweep. If marketplace traffic shape is unknown at deploy time, an operator can shorten retention via a future `OPENHUMAN_AGENTBOX_JOB_RETENTION_SECS` env var. Out of scope for v1.
- **`thread_id` semantics across calls.** v1 trusts the caller's id verbatim. If a marketplace user supplies an id that does not resolve in this deployment's workspace, the `/run` handler still queues the job (the sync handler does not block on thread lookup), and the worker writes `status: "failed"` with `error: "thread not found"`. No cross-tenant leakage because each deployment owns its workspace.
- **Healthcheck contract.** AgentBox docs do not specify one. We expose `/health` and rely on AgentBox console configuring it. If marketplace validation requires a different path, that's a small follow-up.
- **No persistence on restart.** First deployment takes 10–25 minutes per the issue, so restarts are rare — acceptable trade-off. A later PR can swap the in-memory store for a SQLite-backed one without touching the HTTP layer.
- **Approval gate interaction.** The existing approval gate parks interactive chat turns; AgentBox jobs are "background/cron" by nature and should pass through. The worker submission tags the request as background origin so the gate does not park it. To confirm during implementation.

## Acceptance criteria for this PR (code side only)

- `cargo check` and `pnpm rust:check` pass.
- New `src/openhuman/agentbox/` domain compiles and is wired into `build_core_http_router` behind `OPENHUMAN_AGENTBOX_MODE=1`.
- `POST /run` and `GET /jobs/{job_id}` match the contract in this doc (verified by `http_tests.rs`).
- `GMI_MAAS_*` env vars register the OpenAI-compatible provider at startup.
- All new code paths logged with `[agentbox]` / `[agentbox::gmi]` prefixes; secrets never logged.
- Unit + integration test suites pass with `pnpm test:rust`.
- `.env.example` updated.
- `gitbooks/developing/agentbox-deployment.md` added.
- Coverage gate (≥80% on changed lines) satisfied.

Operational acceptance criteria from the issue (marketplace listing visible, container deploys, agent responds, API key stored securely) are explicitly **out of scope of this PR** and remain the operator's checklist.
