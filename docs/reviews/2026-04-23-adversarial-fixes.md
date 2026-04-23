# Adversarial Review — Fix Plan (2026-04-23)

Working branch: `claude/review-openhuman-issues-XG0SR`.
Upstream: `tinyhumansai/openhuman` (fork `jwalin-shah/openhuman` has no open PRs/issues at time of review).

This plan is the concrete, verified follow-up to the adversarial review run on 2026-04-23. It only lists findings that were spot-checked against the actual code. Findings that the initial sweep surfaced but that turned out to be already fixed or too theoretical to action are explicitly excluded below.

## Verification vs. initial sweep

| Item | Status after verification |
| --- | --- |
| RPC `fetch()` has no timeout (`coreRpcClient.ts:154`) | Confirmed |
| Core spawn does not check `try_wait()` before polling (`core_process.rs:161–164`) | Partially confirmed — `try_wait()` is called inside the 100 ms poll loop at line 241, but not immediately after `spawn()`. First exit-check happens after the first sleep. |
| `Arc<Mutex>` on child handle held across `.await` in spawn (`core_process.rs:168–205`) | Confirmed |
| `core_rpc_url` silent fallback (`coreRpcClient.ts:105–112`) | Confirmed |
| CSP wildcard port for localhost in `tauri.conf.json:26` | Confirmed — directly enables the upstream **#812** class of bug |
| MCP transport leaks request handlers on disconnect (`lib/mcp/transport.ts`) | Confirmed (to re-verify when patch is written) |
| Event-bus subscriber panic poisons the loop (`core/event_bus/bus.rs:147`) | **Already fixed** — code at lines 163–180 wraps the handler call in `AssertUnwindSafe(...).catch_unwind()` and logs+continues on panic. Removed from queue. |
| Request ID collision after `Number.MAX_SAFE_INTEGER` calls | Excluded — 2^53 RPC calls per process lifetime is not a realistic failure mode |
| `.unwrap()` in `src/rpc/dispatch.rs:61` | Excluded — it is test code |
| Centralized cancellation for all 97 `tokio::spawn` sites | Deferred — too big for this branch; tracked as follow-up |
| Stale `.claude/rules/*` (upstream **#805**) | Confirmed — `08-frontend-guide.md`, `11-tech-stack-detailed.md`, `16-macos-background-execution.md` contradict CLAUDE.md (e.g. Zustand vs. Redux Toolkit; Telegram MTProto/tray/ticker features that don't exist in code). |

## Queue

Items are ordered by: blast radius × concreteness × low risk of regression. I aim to land them in a single branch, one commit per fix, so each is easy to bisect or revert.

### Security / hardening

1. **[ ] Tighten `tauri.conf.json` CSP** — replace `http://127.0.0.1:*` / `http://localhost:*` wildcard with the actual core port. `default_core_port()` (`app/src-tauri/src/core_process.rs:446`) resolves `OPENHUMAN_CORE_PORT` or falls back to `7788`; CSP should reflect the fallback and any deterministic dev/test ports. Directly addresses upstream **#812** ("screen-share list/thumbnail commands reachable from third-party page origin").

### Reliability / IPC

2. **[ ] RPC fetch timeout** — `callCoreRpc()` must use `AbortController` with a bounded timeout (default 30 s; configurable via `VITE_CORE_RPC_TIMEOUT_MS`) and surface a distinct error message. Today a hung core hangs the UI forever (`app/src/services/coreRpcClient.ts:154`).
3. **[ ] Core spawn: immediate `try_wait()` after `spawn()`** — catch instant-exit children (bad args, missing .so) without eating the first 100 ms sleep. `app/src-tauri/src/core_process.rs:161, 200`.
4. **[ ] Core spawn: drop child lock before awaits** — `self.child.lock().await` is held while the poll loop runs `.await` against the TCP port. Restructure so the lock is released between spawn and port polling. `core_process.rs:168–205`.
5. **[ ] MCP transport: drain pending handlers on disconnect** — on `disconnect`, reject all `requestHandlers` with a synthetic `ConnectionClosed` error so callers unwind instead of waiting on the 30 s timeout. `app/src/lib/mcp/transport.ts`.
6. **[ ] `core_rpc_url` resolution: stop swallowing the failure** — log the caught error via `coreRpcError`, expose a `resolveCoreRpcUrl` diagnostic so the UI can surface "falling back to default URL". `app/src/services/coreRpcClient.ts:105–112`.

### UX / observability

7. **[ ] PersistGate timeout** — wrap `PersistGate` so a rehydration that doesn't complete within 10 s falls through to the app with a toast/banner rather than leaving the user on a permanent loading screen. `app/src/App.tsx:45`.
8. **[ ] RPC errors carry a structured code** — include `{ code, httpStatus, method }` on the thrown error so UI can distinguish timeout / network / JSON-RPC error classes. `coreRpcClient.ts:181–184`.

### Repo hygiene

9. **[ ] Stale `.claude/rules/` pass** — reconcile or delete rules that conflict with CLAUDE.md: at minimum `08-frontend-guide.md` (Zustand/MTProto/ticker), `11-tech-stack-detailed.md` (Zustand, `src-tauri/` path), `16-macos-background-execution.md` ("Outsourced", tray config not in `tauri.conf.json`). Tracks upstream **#805**.

### Explicitly out of scope for this branch

- Cancellation scaffolding for every `tokio::spawn`. Needs a design pass first; separate branch/issue.
- AI config signature verification for remote refresh. Needs product decision on whether remote refresh stays.
- Relay ACL / allowlist on `core_rpc_relay`. Needs explicit scope decision — item (1) closes the main externally reachable gap.
- Deep-link `nonce`/`state` hardening (upstream **#829**). Larger auth-flow change; track upstream.
- Release pipeline reliability (upstream **#828**, **#826**, **#823**, **#843**, **#840**, **#785**). Owned upstream.

## Validation

Before each commit:
- `yarn typecheck` and relevant `yarn test:unit` subset in `app/`.
- `cargo check --manifest-path app/src-tauri/Cargo.toml` for Rust shell changes.
- `cargo check --manifest-path Cargo.toml` for Rust core changes.
- `yarn lint` / `yarn format:check` at the end of the batch.

Final: `yarn typecheck && yarn lint && yarn test:unit && cargo check` across both manifests before pushing.
