# Adversarial Review — Fix Plan (2026-04-23)

Working branch: `claude/review-openhuman-issues-XG0SR`.
Upstream: `tinyhumansai/openhuman` (fork `jwalin-shah/openhuman` has no open PRs/issues at time of review).

This plan is the concrete, verified follow-up to the adversarial review run on 2026-04-23. It only lists findings that were spot-checked against the actual code. Findings that the initial sweep surfaced but that turned out to be already fixed or too theoretical to action are explicitly excluded below.

## Verification vs. initial sweep

| Item | Status after verification |
| --- | --- |
| RPC `fetch()` has no timeout (`coreRpcClient.ts:154`) | Confirmed |
| Core spawn does not check `try_wait()` before polling (`core_process.rs:161–164`) | **Rejected after re-read** — the poll loop at lines 209–254 runs `is_rpc_port_open()` then `try_wait()` *before* the 100 ms sleep, so an instant-exit child is caught within ~250 ms. Not worth changing. |
| `Arc<Mutex>` on child handle held across `.await` in spawn (`core_process.rs:168–205`) | **Rejected after re-read** — `tokio::process::Command::spawn()` is synchronous; no `.await` happens between lock acquire and release in either the spawn block or the poll iteration. Not a real issue. |
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

1. **[~] Tighten `tauri.conf.json` CSP** — deferred. Pinning the core port in static CSP is incompatible with `OPENHUMAN_CORE_PORT` runtime override, and #812 is a capability-level issue (command reachability from 3rd-party origins) that CSP alone wouldn't fix. Needs upstream design.

### Reliability / IPC

2. **[x] RPC fetch timeout** — `callCoreRpc()` uses `AbortController` with a bounded timeout (default 30 s; configurable via `VITE_CORE_RPC_TIMEOUT_MS`) and surfaces a distinct error message. Landed: `app/src/services/coreRpcClient.ts`, `app/src/utils/config.ts`, tests at `app/src/services/__tests__/coreRpcClient.test.ts`, env example updated.
3. ~~Core spawn: immediate `try_wait()` after `spawn()`~~ — rejected on re-read (see table above).
4. ~~Core spawn: drop child lock before awaits~~ — rejected on re-read (see table above).
5. **[x] MCP transport: drain pending handlers on disconnect** — rejects all in-flight `requestHandlers` with `Socket disconnected` (via `disconnect` listener) or `Socket replaced` (via `updateSocket`). Landed: `app/src/lib/mcp/transport.ts` + new test suite `app/src/lib/mcp/__tests__/transport.test.ts`.
6. **[x] `core_rpc_url` resolution: stop swallowing the failure** — both the throw path and the empty-string path now log the underlying misconfig via `coreRpcError`. Landed: `app/src/services/coreRpcClient.ts`.

### UX / observability

7. **[x] PersistGate timeout** — new `PersistRehydrationScreen` preserves the normal splash for the first 10 s, then swaps in a recovery panel that calls `persistor.purge()` + `window.location.reload()`. Landed: `app/src/App.tsx`, `app/src/components/PersistRehydrationScreen.tsx`.
8. **[ ] RPC errors carry a structured code** — deferred to a follow-up. `coreRpcClient.ts` still throws `Error(message)`; richer error codes would churn every call site. Tracked as next-branch work.

### Repo hygiene

9. **[x] Stale `.claude/rules/` pass (upstream #805)** — deleted 7 files that actively contradicted CLAUDE.md (they auto-load into every Claude Code session, so their misinformation is load-bearing context, not just docs):
   - `00-project-vision.md`, `01-project-overview.md` — "crypto community platform, poke.com" framing, mobile targets.
   - `05-platform-setup-android.md`, `06-platform-setup-ios.md` — non-desktop targets are rejected by `compile_error!`.
   - `08-frontend-guide.md` — fabricated features (Telegram MTProto, crypto price ticker, chat with crypto addresses).
   - `11-tech-stack-detailed.md` — claimed Zustand for state mgmt (repo uses Redux Toolkit); wrong directory paths.
   - `16-macos-background-execution.md` — "Outsourced" product name, tray config that isn't in `tauri.conf.json`.

   Rewrote `02-development-commands.md` to match CLAUDE.md (desktop-only, workspace paths, real script names). Trimmed Android/iOS sections out of `10-troubleshooting.md`, fixed `src-tauri/` paths.

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
