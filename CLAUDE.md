# OpenHuman

**AI assistant for communities — React + Tauri v2 desktop app with a Rust core (JSON-RPC / CLI).**

Narrative architecture: [`gitbooks/developing/architecture.md`](gitbooks/developing/architecture.md). Frontend: [`gitbooks/developing/architecture/frontend.md`](gitbooks/developing/architecture/frontend.md). Tauri shell: [`gitbooks/developing/architecture/tauri-shell.md`](gitbooks/developing/architecture/tauri-shell.md). Agent-harness tool surface: [`gitbooks/developing/architecture/agent-harness.md`](gitbooks/developing/architecture/agent-harness.md).

---

## Repository layout

| Path | Role |
| --- | --- |
| **`app/`** | Yarn workspace `openhuman-app`: Vite + React (`app/src/`), Tauri desktop host (`app/src-tauri/`), Vitest tests |
| **`src/`** (root) | Rust lib `openhuman_core` + `openhuman` CLI binary — `core_server`, `openhuman::*` domains, MCP routing |
| **`Cargo.toml`** (root) | Core crate; `cargo build --bin openhuman` produces the sidecar staged by `app`'s `core:stage` |
| **`docs/`** | Remaining deep internals (memory pipeline excalidraws, sentry, telegram-login, etc.). Public contributor docs live in `gitbooks/developing/`. |

Commands assume the **repo root**; `pnpm dev` delegates to the `app` workspace. (Repo migrated from yarn to pnpm — `package.json` enforces pnpm via the `packageManager` field.)

---

## Runtime scope

- **Shipped product**: desktop — Windows, macOS, Linux.
- **Tauri host** (`app/src-tauri`): desktop-only (`compile_error!` for other targets). No Android/iOS branches.
- **Core binary** (`openhuman`): spawned as a **sidecar**; the UI talks to it over HTTP (`core_rpc_relay` + `core_rpc` client), not by duplicating domain logic.

**Where logic lives**
- **Rust core**: business logic, execution, domains, RPC, persistence, CLI. Authoritative.
- **Tauri + React (`app/`)**: UX, screens, navigation, bridging to the core. Presents and orchestrates only.

---

## Commands (from repo root)

```bash
pnpm dev                  # Frontend + Tauri dev
pnpm tauri dev            # Desktop with Tauri (loads env via scripts/load-dotenv.sh)
pnpm build                # Production UI build
pnpm typecheck            # Typecheck (app workspace)
pnpm lint                 # ESLint
pnpm format               # Prettier write
pnpm format:check         # Prettier check
cd app && pnpm core:stage # Stage openhuman binary next to Tauri resources

# Rust — core library + CLI
cargo check --manifest-path Cargo.toml
cargo build --manifest-path Cargo.toml --bin openhuman

# Rust — Tauri shell
cargo check --manifest-path app/src-tauri/Cargo.toml
```

**Tests**: Vitest in `app/` (`pnpm test:unit`, `pnpm test:coverage`); Rust via `cargo test`.
**Quality**: ESLint + Prettier + Husky in `app`.

### Agent debug runners (`scripts/debug/`)

Bounded-output wrappers around the project test runners. Stdout stays summary-sized (so it fits in agent context); full output is teed to `target/debug-logs/<kind>-<suffix>-<timestamp>.log`. Add `--verbose` to also stream raw output. Prefer these over invoking Vitest / WDIO / cargo directly when iterating.

```bash
# Vitest
pnpm debug unit                                    # full suite
pnpm debug unit src/components/Foo.test.tsx        # one file (positional pattern)
pnpm debug unit -t "renders empty state"           # filter by test name
pnpm debug unit Foo -t "renders empty" --verbose

# WDIO E2E (one spec at a time)
pnpm debug e2e test/e2e/specs/smoke.spec.ts
pnpm debug e2e test/e2e/specs/cron-jobs-flow.spec.ts cron-jobs --verbose

# cargo tests (delegates to scripts/test-rust-with-mock.sh)
pnpm debug rust
pnpm debug rust json_rpc_e2e

# Inspect saved logs
pnpm debug logs                  # list 50 most recent
pnpm debug logs last             # print most recent (last 400 lines)
pnpm debug logs unit             # most recent matching prefix "unit"
pnpm debug logs last --tail 100
```

Files: `scripts/debug/{cli,unit,e2e,rust,logs,lib}.sh` plus `README.md`. Entry point is `pnpm debug` (`scripts/debug/cli.sh`).

### Coverage requirement (merge gate)

PRs must meet **≥ 80% coverage on changed lines**. Enforced by [`.github/workflows/coverage.yml`](.github/workflows/coverage.yml) using `diff-cover` over merged Vitest (`app/coverage/lcov.info`) and `cargo-llvm-cov` (core + Tauri shell) lcov outputs. Below the threshold the PR will not merge — add tests for new/changed lines, not just the happy path.

---

## Configuration

- **[`.env.example`](.env.example)** — Rust core, Tauri shell, backend URL, logging, proxy, storage, AI binary overrides. Load via `source scripts/load-dotenv.sh`.
- **[`app/.env.example`](app/.env.example)** — `VITE_*` (core RPC URL, backend URL, Sentry DSN, dev helpers). Copy to `app/.env.local`.

**Frontend config** is centralized in [`app/src/utils/config.ts`](app/src/utils/config.ts). Read `VITE_*` there and re-export — **never** `import.meta.env` directly elsewhere.

**Rust config** uses a TOML `Config` struct (`src/openhuman/config/schema/types.rs`) with env overrides (`src/openhuman/config/schema/load.rs`).

---

## Testing

### Unit (Vitest)

- Co-locate as `*.test.ts` / `*.test.tsx` under `app/src/**`.
- Config: `app/test/vitest.config.ts`; setup: `app/src/test/setup.ts`.
- Run: `pnpm test:unit`, `pnpm test:coverage`.
- Prefer behavior over implementation. Use helpers in `app/src/test/`. No real network, no time flakes.

### Shared mock backend

Used by both unit and Rust tests.
- Core: `scripts/mock-api-core.mjs` · server: `scripts/mock-api-server.mjs` · E2E wrapper: `app/test/e2e/mock-server.ts`.
- Admin: `GET /__admin/health`, `POST /__admin/reset`, `POST /__admin/behavior`, `GET /__admin/requests`.
- Run manually: `pnpm mock:api`.

### E2E (WDIO — dual platform)

Full guide: [`gitbooks/developing/e2e-testing.md`](gitbooks/developing/e2e-testing.md).
- **Linux (CI)**: `tauri-driver` (WebDriver :4444).
- **macOS (local)**: Appium Mac2 (XCUITest :4723) on the `.app` bundle.
- Specs: `app/test/e2e/specs/*.spec.ts`. Helpers in `app/test/e2e/helpers/`. Config: `app/test/wdio.conf.ts`.

```bash
pnpm test:e2e:build
bash app/scripts/e2e-run-spec.sh test/e2e/specs/smoke.spec.ts smoke
pnpm test:e2e:all:flows
docker compose -f e2e/docker-compose.yml run --rm e2e   # Linux E2E on macOS
```

Use `element-helpers.ts` (`clickNativeButton`, `waitForWebView`, `clickToggle`) — never raw `XCUIElementType*`. Assert UI outcomes and mock effects.

### Deterministic core-sidecar reset

`app/scripts/e2e-run-spec.sh` creates and cleans a temp `OPENHUMAN_WORKSPACE` by default. `OPENHUMAN_WORKSPACE` redirects core config + storage away from `~/.openhuman`.

### Rust tests with mock

```bash
pnpm test:rust
bash scripts/test-rust-with-mock.sh --test json_rpc_e2e
```

---

## Frontend (`app/src/`)

**Provider chain** (`App.tsx`):
`Redux` → `PersistGate` → `UserProvider` → `SocketProvider` → `AIProvider` → `SkillProvider` → `HashRouter` → `AppRoutes`.

**State** (`store/`): Redux Toolkit slices — auth, user, socket, ai, skills, team, etc. Prefer Redux (persisted where configured) over ad-hoc `localStorage`.

**Services** (`services/`): singletons — `apiClient`, `socketService`, `coreRpcClient` (HTTP bridge to core), domain `api/*` clients.

**MCP** (`lib/mcp/`): JSON-RPC transport, validation, types over Socket.io. Tooling is driven by the backend + skills system.

**Routing** (`AppRoutes.tsx`): hash routes `/`, `/onboarding`, `/mnemonic`, `/home`, `/intelligence`, `/skills`, `/conversations`, `/invites`, `/agents`, `/settings/*`. No `/login`.

**AI config**: bundled prompts in `src/openhuman/agent/prompts/` (also bundled via `app/src-tauri/tauri.conf.json` `resources`). Loaders in `app/src/lib/ai/` use `?raw` imports, optional remote fetch, and `ai_get_config` / `ai_refresh_config` in Tauri.

---

## Tauri shell (`app/src-tauri/`)

Thin desktop host: window management, daemon health, **core process lifecycle** (`core_process`, `CoreProcessHandle`), **JSON-RPC relay** (`core_rpc_relay`, `core_rpc`).

Registered IPC (see [`gitbooks/developing/architecture/tauri-shell.md`](gitbooks/developing/architecture/tauri-shell.md)): `greet`, `write_ai_config_file`, `ai_get_config`, `ai_refresh_config`, `core_rpc_relay`, window commands, `openhuman_*` daemon helpers.

### CEF child webviews — no new JS injection

Embedded provider webviews (`acct_*`, loading third-party origins like `web.telegram.org`, `linkedin.com`, `slack.com`, …) **must not** grow any new JavaScript injection. Do not add new `.js` files under `app/src-tauri/src/webview_accounts/`, do not append new blocks to `build_init_script` / `RUNTIME_JS`, and do not dispatch scripts via CDP `Page.addScriptToEvaluateOnNewDocument` / `Runtime.evaluate` for these webviews. The migrated providers (whatsapp, telegram, slack, discord, browserscan) load with **zero** injected JS under CEF by design — all scraping and observability runs natively via CDP in the per-provider scanner modules, and anything host-controlled that runs inside a third-party origin is a scraping/attack-surface liability.

New behavior for these webviews lives in:

- **CEF handlers** — `on_navigation`, `on_new_window`, `LoadHandler::OnLoadStart`, `CefRequestHandler::*` (wired in `webview_accounts/mod.rs`).
- **CDP from the scanner side** — `Network.*`, `Emulation.*`, `Input.*`, `Page.*` driven by the per-provider `*_scanner/` modules.
- **Rust-side notification/IPC hooks** — never cross into the renderer.

If a feature truly cannot be built this way (e.g. intercepting a click the page's JS preventDefaults), the correct answer is to **surface the limitation**, not to ship an init script. Legacy injection that already exists for non-migrated providers (`gmail`, `linkedin`, `google-meet` recipe files plus the `runtime.js` bridge) is grandfathered but should shrink, not grow.

Watch out for Tauri plugins that inject JS by default. `tauri-plugin-opener` ships `init-iife.js` (a global click listener that calls `plugin:opener|open_url` via HTTP-IPC) unless you build it with `.open_js_links_on_click(false)`. Any new plugin added to `app/src-tauri/src/lib.rs` must be audited for a `js_init_script` call — if found, opt out or configure around it.

---

## Rust core (`src/`)

- **`openhuman/`** — Domain logic (memory, channels, config, cron, skills, webhooks, …). RPC controllers in per-domain `rpc.rs`; use `RpcOutcome<T>` per [`AGENTS.md`](AGENTS.md).
- **Module layout rule**: new functionality goes in a **dedicated subdirectory** (`openhuman/<domain>/mod.rs` + siblings). **Do not** add new standalone `*.rs` files at `src/openhuman/` root.
- **Controller schema contract**: shared types in `src/core/mod.rs` (`ControllerSchema`, `FieldSchema`, `TypeSchema`).
- **Domain schema files**: per-domain `schemas.rs` (e.g. `src/openhuman/cron/schemas.rs`), exported from domain `mod.rs`.
- **Controller-only exposure**: expose features to CLI and JSON-RPC via the controller registry. **Do not** add domain branches in `src/core/cli.rs` / `src/core/jsonrpc.rs`.
- **Light `mod.rs`**: keep domain `mod.rs` export-focused. Operational code in `ops.rs`, `store.rs`, `types.rs`, etc.
- **`core_server/`** — Transport only: Axum/HTTP, JSON-RPC envelope, CLI parsing, dispatch. No heavy logic.

### Controller migration checklist

- `src/openhuman/<domain>/mod.rs`: add `mod schemas;`, re-export `all_controller_schemas as all_<domain>_controller_schemas` and `all_registered_controllers as all_<domain>_registered_controllers`.
- `src/openhuman/<domain>/schemas.rs` defines `schemas`, `all_controller_schemas`, `all_registered_controllers`, and `handle_*` fns delegating to domain `rpc.rs`.
- Wire exports into `src/core/all.rs`. Remove migrated branches from `src/rpc/dispatch.rs`.

### Event bus (`src/core/event_bus/`)

Typed pub/sub + in-process typed request/response. Both singletons — use module-level functions; never construct `EventBus` / `NativeRegistry` directly.

- **Broadcast** (`publish_global` / `subscribe_global`) — fire-and-forget. Many subscribers, no return.
- **Native request/response** (`register_native_global` / `request_native_global`) — one-to-one typed dispatch keyed by method string. Zero serialization — trait objects, `mpsc::Sender`, `oneshot::Sender` pass through unchanged. Internal-only; JSON-RPC-facing work goes through `src/core/all.rs`.

Core types (all in `src/core/event_bus/`):

| Type | File | Purpose |
| --- | --- | --- |
| `DomainEvent` | `events.rs` | `#[non_exhaustive]` enum of all cross-module events |
| `EventBus` | `bus.rs` | Singleton over `tokio::sync::broadcast`; ctor is `pub(crate)` |
| `NativeRegistry` / `NativeRequestError` | `native_request.rs` | Typed request/response registry by method name |
| `EventHandler` | `subscriber.rs` | Async trait with optional `domains()` filter |
| `SubscriptionHandle` | `subscriber.rs` | RAII — drops cancel the subscriber |
| `TracingSubscriber` | `tracing.rs` | Built-in debug logger |

Singleton API: `init_global(capacity)`, `publish_global(event)`, `subscribe_global(handler)`, `register_native_global(method, handler)`, `request_native_global(method, req)`, `global()` / `native_registry()`.

Domains: `agent`, `memory`, `channel`, `cron`, `skill`, `tool`, `webhook`, `system`.

Each domain owns a `bus.rs` with its `EventHandler` impls — e.g. `cron/bus.rs` (`CronDeliverySubscriber`), `webhooks/bus.rs` (`WebhookRequestSubscriber`), `channels/bus.rs` (`ChannelInboundSubscriber`). Convention: `<Purpose>Subscriber` + `name()` returning `"<domain>::<purpose>"`.

**Adding events**: add variants to `DomainEvent`, extend the `domain()` match, create `<domain>/bus.rs`, register subscribers at startup, publish via `publish_global`.

**Adding a native handler**: define request/response types in the domain (owned fields, `Arc`s, channels — not borrows; `Send + 'static`, not `Serialize`). Register at startup keyed by `"<domain>.<verb>"`. Callers dispatch via `request_native_global`.

**Tests**: re-register the same method to override; or construct a fresh `NativeRegistry::new()` for isolation.

---

## Design

Premium, calm visual language — ocean primary `#4A83DD`, sage / amber / coral semantics, Inter + Cabinet Grotesk + JetBrains Mono, Tailwind with custom radii/spacing/shadows. Implementation tokens live in [`app/tailwind.config.js`](app/tailwind.config.js).

## Shell vs app code

Tauri/Rust in the shell is a **delivery vehicle** (windowing, process lifecycle, IPC). Keep UI behavior and product logic in TypeScript/React (`app/`). Only grow Rust in the shell for hard platform/security reasons.

## Git workflow

- Issues and PRs on upstream **[tinyhumansai/openhuman](https://github.com/tinyhumansai/openhuman)** — not a fork — unless explicitly told otherwise.
- Issue templates: [`.github/ISSUE_TEMPLATE/feature.md`](.github/ISSUE_TEMPLATE/feature.md), [`.github/ISSUE_TEMPLATE/bug.md`](.github/ISSUE_TEMPLATE/bug.md). PR template: [`.github/PULL_REQUEST_TEMPLATE.md`](.github/PULL_REQUEST_TEMPLATE.md). AI-authored text should follow them verbatim.
- PRs target **`main`**.
- **Push branches to `origin` (the user's fork — `senamakel/openhuman`), never to `upstream` (`tinyhumansai/openhuman`).** PRs are still opened against `tinyhumansai/openhuman:main`, but with `--head senamakel:<branch>` so the source is the fork. Direct pushes to upstream pollute its branch list and skip code-review boundaries. Treat the `upstream` remote as fetch-only.
- **When the user asks you to push or open a PR, resolve blockers and push — don't prompt for permission.** If a pre-push hook fails on something unrelated to your changes (e.g. pre-existing breakage on `main` in code you didn't touch), push with `--no-verify` and call it out in the PR body. If the hook fails on your own changes, fix them and push again. Don't ask the user whether to bypass — just do the right thing and tell them what you did.

---

## Coding philosophy

- **Unix-style modules**: small, sharp-responsibility units composed through clear boundaries.
- **Tests before the next layer**: ship unit tests for new/changed behavior before stacking features. Untested code is incomplete.
- **Docs with code**: new/changed behavior ships with matching rustdoc / code comments; update `AGENTS.md` or architecture docs when rules or user-visible behavior change.

---

## Debug logging (must follow)

- Default to **verbose diagnostics** on new/changed flows so issues are easy to trace end-to-end.
- Log entry/exit, branches, external calls, retries/timeouts, state transitions, errors.
- Use stable grep-friendly prefixes (`[domain]`, `[rpc]`, `[ui-flow]`) and correlation fields (request IDs, method names, entity IDs).
- Rust: `log` / `tracing` at `debug` / `trace`. `app/`: namespaced `debug` + dev-only detail.
- **Never** log secrets or full PII — redact.
- Changes lacking diagnosis logging are incomplete.

---

## Feature design workflow

Specify → prove in Rust → prove over RPC → surface in the UI → test.

1. **Specify against the current codebase** — ground in existing domains, controller/registry patterns, JSON-RPC naming (`openhuman.<namespace>_<function>`). No parallel architectures.
2. **Implement in Rust** — domain logic under `src/openhuman/<domain>/`, schemas + handlers in the registry, unit tests until correct in isolation.
3. **JSON-RPC E2E** — extend [`tests/json_rpc_e2e.rs`](tests/json_rpc_e2e.rs) / [`scripts/test-rust-with-mock.sh`](scripts/test-rust-with-mock.sh) so RPC methods match what the UI will call.
4. **UI in Tauri app** — React screens/state using `core_rpc_relay` / `coreRpcClient`. Keep rules in the core.
5. **App unit tests** — Vitest.
6. **App E2E** — desktop specs for user-visible flows.

**Capability catalog**: when a change adds/removes/renames a user-facing feature, update `src/openhuman/about_app/` in the same work.

**Planning rule**: up front, define the **E2E scenarios (core RPC + app)** that cover the full intended scope — happy paths, failure modes, auth gates, regressions. Not testable end-to-end ⇒ incomplete spec or too-large cut.

---

## Key patterns

- **File size**: prefer ≤ ~500 lines; split growing modules.
- **Pre-merge** (code changes): Prettier, ESLint, `tsc --noEmit` in `app/`; `cargo fmt` + `cargo check` for changed Rust.
- **No dynamic imports** in production `app/src` code — static `import` / `import type` only. No `import()`, `React.lazy(() => import(...))`, `await import(...)`. For heavy optional paths, use a static import and guard the call site with `try/catch` or a runtime check. *Exceptions*: Vitest harness patterns in `*.test.ts` / `__tests__` / `test/setup.ts`; ambient `typeof import('…')` in `.d.ts`; config files (e.g. `tailwind.config.js` JSDoc).
- **Dual socket sync**: when changing the realtime protocol, keep `socketService` / MCP transport aligned with core socket behavior (see `gitbooks/developing/architecture.md` dual-socket section).

---

## Platform notes

- **Vendored CEF-aware `tauri-cli`**: runtime is CEF; only the vendored CLI at `app/src-tauri/vendor/tauri-cef/crates/tauri-cli` bundles Chromium into `Contents/Frameworks/`. Stock `@tauri-apps/cli` produces a broken bundle (panic in `cef::library_loader::LibraryLoader::new`). `pnpm dev:app` and all `cargo tauri` scripts call `pnpm tauri:ensure` which runs [`scripts/ensure-tauri-cli.sh`](scripts/ensure-tauri-cli.sh). If overwritten, reinstall with `cargo install --locked --path app/src-tauri/vendor/tauri-cef/crates/tauri-cli`.
- **macOS deep links**: often require a built `.app` bundle, not just `tauri dev`.
- **Tauri environment guard**: use `isTauri()` (from `app/src/services/webviewAccountService.ts`) or wrap `invoke(...)` in `try/catch`; do not check `window.__TAURI__` directly — it is not present at module load and bypasses the established wrapper contract.
- **Core sidecar**: must be staged so `core_rpc` can reach the `openhuman` binary (see `scripts/stage-core-sidecar.mjs`).
