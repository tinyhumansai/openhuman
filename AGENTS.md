# OpenHuman

**AI assistant for communities — React + Tauri v2 desktop app with a Rust core (JSON-RPC / CLI) embedded in-process.**

This file orients contributors and coding agents. Authoritative narrative architecture: [`gitbooks/developing/architecture.md`](gitbooks/developing/architecture.md). Frontend layout: [`gitbooks/developing/architecture/frontend.md`](gitbooks/developing/architecture/frontend.md). Tauri shell: [`gitbooks/developing/architecture/tauri-shell.md`](gitbooks/developing/architecture/tauri-shell.md).

---

## Repository layout

| Path                    | Role                                                                                                                                                                                                        |
| ----------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **`app/`**              | pnpm workspace **`openhuman-app`** (v0.53.45): Vite + React (`app/src/`), Tauri desktop host (`app/src-tauri/`), Vitest tests                                                                              |
| **Repo root `src/`**    | Rust library crate **`openhuman`** (lib name) with **`openhuman-core`** CLI binary entrypoint (`src/main.rs`) — `src/core/` (transport: HTTP/JSON-RPC, CLI, dispatch, auth, event bus), `src/openhuman/*` domains. The QuickJS skills runtime has been removed; `src/openhuman/skills/` is metadata-only now. |
| **Skills registry**     | **[`tinyhumansai/openhuman-skills`](https://github.com/tinyhumansai/openhuman-skills)** on GitHub — canonical skill packages and TS build; not vendored in this tree.                                       |
| **`Cargo.toml`** (root) | Core crate; `cargo build --bin openhuman-core` produces the CLI binary. Helper binaries: `slack-backfill`, `gmail-backfill-3d` in `src/bin/`.                                                              |
| **`docs/`**             | Architecture and deep-internal references                                                                                                                                                                    |
| **`gitbooks/developing/`** | Public contributor docs — frontend, Tauri shell, testing, release, agent harness, CEF, observability                                                                                                       |

Commands in documentation assume the **repo root** unless noted: `pnpm dev` runs Vite-only inside the `app` workspace; `pnpm dev:app` runs the full Tauri desktop dev (CEF runtime).

**Skills registry:** Skill sources and the bundler live in **[github.com/tinyhumansai/openhuman-skills](https://github.com/tinyhumansai/openhuman-skills)**. The desktop app's skills catalog defaults to that GitHub slug; override with `VITE_SKILLS_GITHUB_REPO` (see [`app/src/utils/config.ts`](app/src/utils/config.ts)). Note: since the QuickJS runtime was removed, the desktop app no longer *executes* skill packages — it only discovers, installs metadata, and renders catalog entries. Skill execution surfaces are being rebuilt; check the current domain modules before assuming a skill can run end-to-end.

---

## Runtime scope

- **Shipped product**: desktop — Windows, macOS, Linux (see [`gitbooks/developing/architecture.md`](gitbooks/developing/architecture.md) "Platform reach").
- **Tauri host** (`app/src-tauri`): **desktop-only**. Do not add Android/iOS branches.
- **Core runs in-process** as a tokio task inside the Tauri host (sidecar removed in PR #1061). The host owns its lifetime via `core_process::CoreProcessHandle` in `app/src-tauri/src/core_process.rs`. Frontend RPC still goes over HTTP to `http://127.0.0.1:<port>/rpc` authenticated with a per-launch hex bearer in `OPENHUMAN_CORE_TOKEN`; the Tauri command `core_rpc_token` exposes it to the renderer. Set `OPENHUMAN_CORE_REUSE_EXISTING=1` to attach to an externally-started `openhuman-core` process for debugging.

**Where logic lives**

- **Rust (`openhuman` / repo root `src/`)**: **Business logic and execution**—domains, RPC, persistence, CLI behavior. Authoritative.
- **Tauri + React (`app/`)**: **Interaction and UX**—screens, navigation, input, accessibility, windowing, and bridging to the in-process core. The shell presents and orchestrates; it does not duplicate core business rules.

---

## Commands (from repository root)

```bash
# Vite dev only (no Tauri host)
pnpm dev

# Full desktop with Tauri/CEF (loads env via scripts/load-dotenv.sh)
pnpm dev:app

# Production UI build (app workspace)
pnpm build

# Typecheck / lint / format
pnpm typecheck          # alias for `pnpm compile` (tsc --noEmit)
pnpm lint
pnpm format             # Prettier + cargo fmt
pnpm format:check

# `pnpm core:stage` is a no-op — sidecar removed in PR #1061; core is in-process.

# Skills — author / build in the registry repo (tinyhumansai/openhuman-skills).
# There are no `skills:build` / `skills:watch` scripts in this repo's app workspace.

# Rust — core library + CLI (repo root)
cargo check --manifest-path Cargo.toml
cargo build --manifest-path Cargo.toml --bin openhuman-core

# Rust — Tauri shell only
cargo check --manifest-path app/src-tauri/Cargo.toml
pnpm rust:check         # same as above

# whisper-rs / llama.cpp on macOS Tahoe (Apple Silicon) fail with `-mcpu=native`.
# Workaround for `cargo check`/`cargo test`:
GGML_NATIVE=OFF cargo check --manifest-path Cargo.toml

# PR maintenance
pnpm pr:sync-main --help        # inspect options
pnpm pr:sync-main               # dry-run: scan open PRs targeting main
pnpm pr:sync-main --execute     # merge latest main into each matching PR branch and push
```

**Tests**: Vitest in `app/` (`pnpm test`, `pnpm test:coverage`); Rust via `pnpm test:rust` (runs `scripts/test-rust-with-mock.sh`).

**Quality**: ESLint + Prettier + Husky in the `app` workspace. Pre-push hook runs `pnpm rust:check`. Use `--no-verify` only for unrelated pre-existing breakage and call it out in the PR body.

### Codex web / Linear-launched PR checklist

Before opening AI-authored PRs from Codex web sessions or Linear-launched implementation agents, follow [`docs/agent-workflows/codex-pr-checklist.md`](docs/agent-workflows/codex-pr-checklist.md).

This checklist is required for remote agents because OpenHuman has several merge gates that are easy to miss in partial environments: Prettier, Rust formatting, TypeScript typecheck, focused Vitest coverage, controller dispatch parity, and Tauri vendored dependency availability. If a command cannot run in the remote environment, the PR body must report the exact blocked command and error instead of claiming validation passed.

### Agent debug runners (`scripts/debug/`)

Use these wrappers instead of invoking Vitest / WDIO / cargo directly when iterating — they keep stdout summary-sized and tee full output to `target/debug-logs/<kind>-<suffix>-<timestamp>.log`. Add `--verbose` to also stream raw output. See [`scripts/debug/README.md`](scripts/debug/README.md).

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

Files: `scripts/debug/{cli,unit,e2e,rust,logs,lib}.sh`. Entry point: `pnpm debug` (`scripts/debug/cli.sh`).

### Coverage requirement (merge gate)

PRs must meet **≥ 80% coverage on changed lines**. Enforced by [`.github/workflows/coverage.yml`](.github/workflows/coverage.yml) via `diff-cover` over merged Vitest + `cargo-llvm-cov` (core + Tauri shell) lcov outputs. Below the threshold the PR will not merge. Run `pnpm test:coverage` and `pnpm test:rust` locally; add tests for new/changed lines (happy path + at least one failure / edge case).

---

## Configuration

Environment variables are documented in two `.env.example` files:

- **[`.env.example`](.env.example)** (repo root) — Rust core, Tauri shell, backend URL, logging, proxy, storage, web search, local AI binary overrides. Loaded via `source scripts/load-dotenv.sh`.
- **[`app/.env.example`](app/.env.example)** — Frontend `VITE_*` vars (core RPC URL, backend URL, Sentry DSN, skills repo, dev helpers). Copy to `app/.env.local` for local overrides.

**Frontend config** is centralized in [`app/src/utils/config.ts`](app/src/utils/config.ts). All `VITE_*` env vars should be read there and re-exported — do not read `import.meta.env` directly in other files.

**Rust config** uses a TOML-based `Config` struct (`src/openhuman/config/schema/types.rs`) with env var overrides applied in `src/openhuman/config/schema/load.rs`. Env vars override config file values at runtime (e.g. `OPENHUMAN_API_URL` overrides `config.api_url`).

---

## Testing Guide (Unit + E2E)

### Unit tests (Vitest)

- **Where tests live**: co-locate as `*.test.ts` / `*.test.tsx` under `app/src/**`.
- **Runner/config**: Vitest with `app/test/vitest.config.ts` and shared setup in `app/src/test/setup.ts`.
- **Run**:

```bash
pnpm test:unit
pnpm test:coverage
```

- **Authoring rules**:
  - Prefer testing behavior over implementation details.
  - Use existing helpers from `app/src/test/` (`test-utils.tsx`, shared mock backend) before adding new harness code.
  - Keep tests deterministic: avoid real network calls, time-sensitive flakes, or hidden global state.

### Shared mock backend (app + Rust tests)

- **Core implementation**: `scripts/mock-api-core.mjs`
- **Standalone server entrypoint**: `scripts/mock-api-server.mjs`
- **E2E wrapper**: `app/test/e2e/mock-server.ts`
- **Vitest unit setup**: `app/src/test/setup.ts` starts the shared mock server by default on `http://127.0.0.1:5005`.

Key admin endpoints:

- `GET /__admin/health`
- `POST /__admin/reset`
- `POST /__admin/behavior`
- `GET /__admin/requests`

Run manually:

```bash
pnpm mock:api
curl -s http://127.0.0.1:18473/__admin/health
```

### E2E tests (WDIO — dual platform)

Full guide: [`gitbooks/developing/e2e-testing.md`](gitbooks/developing/e2e-testing.md).

Two automation backends:
- **Linux (CI default)**: `tauri-driver` (WebDriver, port 4444) — drives the debug binary directly
- **macOS (local dev)**: Appium Mac2 (XCUITest, port 4723) — drives the `.app` bundle

- **Where specs live**: `app/test/e2e/specs/*.spec.ts`
- **Shared harness**:
  - Platform detection: `app/test/e2e/helpers/platform.ts`
  - Element helpers: `app/test/e2e/helpers/element-helpers.ts`
  - Deep link helpers: `app/test/e2e/helpers/deep-link-helpers.ts`
  - App lifecycle: `app/test/e2e/helpers/app-helpers.ts`
  - Mock backend: `app/test/e2e/mock-server.ts`
  - WDIO config: `app/test/wdio.conf.ts` (auto-detects platform)

- **Build + run**:

```bash
# Build app + stage core sidecar (detects macOS vs Linux automatically)
pnpm test:e2e:build

# Run one spec
bash app/scripts/e2e-run-spec.sh test/e2e/specs/smoke.spec.ts smoke

# Run all flow specs
pnpm test:e2e:all:flows

# Docker on macOS (run Linux E2E locally)
docker compose -f e2e/docker-compose.yml run --rm e2e
```

- **Authoring rules**:
  - Ensure each spec is runnable in isolation.
  - Use helpers from `element-helpers.ts` — never use raw `XCUIElementType*` selectors in specs.
  - Use `clickNativeButton()`, `hasAppChrome()`, `waitForWebView()`, `clickToggle()` for cross-platform element interaction.
  - Assert both UI outcomes and backend/mock effects when relevant.
  - Add failure diagnostics (request logs, `dumpAccessibilityTree()`) for faster debugging by agents.

### Deterministic core-sidecar reset

By default, `app/scripts/e2e-run-spec.sh` creates and cleans a temp `OPENHUMAN_WORKSPACE`
automatically when the variable is not provided.

If you need a fixed workspace for debugging, provide one explicitly:

```bash
export OPENHUMAN_WORKSPACE="$(mktemp -d)"
pnpm test:e2e:build
bash app/scripts/e2e-run-spec.sh test/e2e/specs/smoke.spec.ts smoke
rm -rf "$OPENHUMAN_WORKSPACE"
```

- `OPENHUMAN_WORKSPACE` redirects core config + workspace storage away from `~/.openhuman`.
- Default reset strategy:
  - Rebuild/stage sidecar once per E2E run (`pnpm test:e2e:build`).
  - Isolate state per test case with a fresh temp workspace (default behavior in `e2e-run-spec.sh`).

### Rust tests with mock backend

Use the shared mock backend runner so Rust unit/integration tests get deterministic API behavior:

```bash
pnpm test:rust
# or targeted
bash scripts/test-rust-with-mock.sh --test json_rpc_e2e
```

Example per-test-case pattern inside a harness script:

```bash
run_case() {
  export OPENHUMAN_WORKSPACE="$(mktemp -d)"
  bash app/scripts/e2e-run-spec.sh "$1" "$2"
  rm -rf "$OPENHUMAN_WORKSPACE"
}
```

- **Rust test file naming**: when extracting Rust tests out of an implementation file, prefer a sibling `*_test.rs` file wired in with `#[cfg(test)] #[path = "..._test.rs"] mod tests;`. Do not create ad hoc `_test/` or `_tests/` directories for single-module Rust tests unless a broader multi-file test fixture truly requires a directory.

### Test authoring checklist

- Add/update unit tests for logic changes before stacking additional features.
- Add/update E2E coverage for user-visible flows and cross-process integration behavior.
- Keep new tests independent, deterministic, and debuggable from logs alone.
- When touching core/sidecar behavior, validate both:
  - `pnpm test:unit`
  - targeted E2E spec(s) via `app/scripts/e2e-run-spec.sh`

---

## Frontend (`app/src/`)

### Provider chain (`app/src/App.tsx`)

Order matters for auth and realtime:

`Sentry.ErrorBoundary` → `Redux Provider` → `PersistGate` (with `PersistRehydrationScreen`) → `BootCheckGate` → **`CoreStateProvider`** → **`SocketProvider`** → **`ChatRuntimeProvider`** → `HashRouter` → `CommandProvider` → `ServiceBlockingGate` → `AppShell` (`AppRoutes` + `BottomTabBar` + `AppWalkthrough` + `MascotFrameProducer`).

`CoreStateProvider` owns auth: session tokens are NOT in redux-persist; they live in the in-process core and are fetched via `fetchCoreAppSnapshot()` RPC. There is no `UserProvider`, `AIProvider`, `SkillProvider`, or `TelegramProvider`.

### State (`app/src/store/`)

Redux Toolkit slices: `accounts`, `channelConnections`, `chatRuntime`, `coreMode`, `deepLinkAuth`, `mascot`, `notification`, `providerSurface`, `socket` (+ `socketSelectors`), `thread`. Plus `userScopedStorage` for per-user persistence keys. Prefer Redux (and redux-persist where configured) over ad hoc `localStorage` for app state. Documented exception: ephemeral UI state like upsell-banner dismiss flags use `localStorage` with the `openhuman:upsell:` prefix.

### Services (`app/src/services/`)

Singleton-style modules: `apiClient`, `socketService`, `coreRpcClient`, `coreCommandClient`, `chatService`, `analytics`, `notificationService`, `webviewAccountService`, `daemonHealthService`, `meetCallService`, `memorySyncService`, `bootCheckService`, `walletApi`, plus domain `api/*` clients. Always call the in-process core via `invoke('core_rpc_relay', ...)` — never raw `fetch()` (CORS preflight) or `callCoreRpc()` for service-status calls (socket may not be connected yet).

### MCP (`app/src/lib/mcp/`)

Transport, validation, and types for JSON-RPC-style messaging over Socket.io. Tooling for agents is driven by the **skills** catalog metadata + backend tool registry; see `agentToolRegistry.ts` and core RPC. (Skill execution itself moved out-of-process when the QuickJS runtime was removed.)

### Routing (`app/src/AppRoutes.tsx`, HashRouter)

`/` (Welcome, public), `/onboarding/*`, `/home`, `/human`, `/intelligence`, `/skills`, `/chat` (unified agent + connected web apps — replaces the old `/conversations` and `/accounts` routes), `/channels`, `/invites`, `/notifications`, `/rewards`, `/webhooks` → redirect to `/settings/webhooks-triggers`, `/settings/*`. Default `*` → `DefaultRedirect`. There is **no** `/login`, **no** `/mnemonic` (Recovery Phrase moved to Settings panel), **no** `/agents`, **no** `/conversations`.

### AI configuration

Bundled prompts live under **`src/openhuman/agent/prompts/`** at the **repository root** (also bundled via `app/src-tauri/tauri.conf.json` `resources`). Loaders under `app/src/lib/ai/` use `?raw` imports, optional remote fetch, and in Tauri **`ai_get_config` / `ai_refresh_config`** for packaged content.

---

## Tauri shell (`app/src-tauri/`)

Thin desktop host. Top-level modules: `core_process`, `core_rpc`, `cdp`, `cef_preflight`, `cef_profile`, `dictation_hotkeys`, `file_logging`, `mascot_native_window`, `native_notifications`, `notification_settings`, `process_kill`, `process_recovery`, `screen_capture`, `window_state`, plus per-provider scanners (`discord_scanner`, `gmessages_scanner`, `imessage_scanner`, `meet_scanner`, `slack_scanner`, `telegram_scanner`, `whatsapp_scanner`), `meet_audio` / `meet_call` / `meet_video`, `fake_camera`, `webview_accounts`, `webview_apis`.

**Core lifecycle**: `core_process::CoreProcessHandle` spawns the in-process JSON-RPC server and authenticates inbound RPC with a hex bearer (`OPENHUMAN_CORE_TOKEN`). Stale-listener policy (#1130): on conflict the handle probes `GET /`, decides if the listener is an OpenHuman core, then `kill_pid_term` → `kill_pid_force` with PID revalidation guarding against PID reuse. `restart_core_process` / `start_core_process` Tauri commands let the frontend cycle it for updates.

Registered IPC commands (see [`gitbooks/developing/architecture/tauri-shell.md`](gitbooks/developing/architecture/tauri-shell.md)) include `greet`, `write_ai_config_file`, `ai_get_config`, `ai_refresh_config`, `core_rpc_relay`, `core_rpc_token`, `start_core_process`, `restart_core_process`, window commands, and `openhuman_*` daemon helpers.

Deep link plugin is registered where supported; behavior is platform-specific (see platform notes below).

---

## Rust core (repo root `src/`)

- **`src/openhuman/`** — Domain logic. Current domains: `about_app`, `accessibility`, `agent`, `app_state`, `approval`, `autocomplete`, `billing`, `channels`, `composio`, `config`, `context`, `cost`, `credentials`, `cron`, `doctor`, `embeddings`, `encryption`, `health`, `heartbeat`, `integrations`, `learning`, `local_ai`, `meet`, `meet_agent`, `memory`, `migration`, `node_runtime`, `notifications`, `overlay`, `people`, `prompt_injection`, `provider_surfaces`, `providers`, `redirect_links`, `referral`, `routing`, `scheduler_gate`, `screen_intelligence`, `security`, `service`, `skills`, `socket`, `subconscious`, `team`, `text_input`, `threads`, `tokenjuice`, `tool_timeout`, `tools`, `tree_summarizer`, `update`, `voice`, `wallet`, `webhooks`, `webview_accounts`, `webview_apis`, `webview_notifications`. RPC controllers in per-domain `rpc.rs`; use **`RpcOutcome<T>`** pattern (see "RPC Controller Pattern" below).
- **`src/openhuman/` module layout**: **New** functionality must live in a **dedicated subdirectory** (e.g. `openhuman/my_domain/mod.rs` plus related files, or a new subfolder under an existing domain). Do **not** add new standalone `*.rs` files directly at `src/openhuman/` root (`dev_paths.rs` and `util.rs` are grandfathered).
- **Controller schema contract**: Shared controller metadata types live in `src/core/types.rs` / `src/core/mod.rs` (`ControllerSchema`, `FieldSchema`, `TypeSchema`) and are consumed by adapters (RPC/CLI).
- **Domain schema files**: For each domain, define controller schema metadata in a dedicated module inside the domain folder (example: `src/openhuman/cron/schemas.rs`) and export from the domain `mod.rs`.
- **Controller-only exposure rule**: Expose domain functionality to **CLI and JSON-RPC through the controller registry** (`schemas.rs` + registered handlers wired into `src/core/all.rs`). Do **not** add domain-specific branches in `src/core/cli.rs` or `src/core/jsonrpc.rs`.
- **Light `mod.rs` rule**: Keep domain `mod.rs` files light and export-focused. Put operational code in sibling files (`ops.rs`, `store.rs`, `schedule.rs`, `types.rs`, `bus.rs`).
- **`src/core/`** — Transport only: Axum/HTTP, JSON-RPC envelope, CLI parsing, **dispatch** (`src/core/dispatch.rs`), auth, observability, event bus. **No** heavy business logic here. (Older docs that say `core_server` mean this directory; there is no `src/core_server/`.)
- **Layering**: Implementation in `openhuman::<domain>/`, controllers in `openhuman::<domain>/rpc.rs`, routes/dispatch in `src/core/`.

The previous QuickJS / `rquickjs` skills runtime has been removed. `src/openhuman/skills/` now contains metadata helpers only (`ops_create`, `ops_discover`, `ops_install`, `ops_parse`, `inject`, `schemas`, `types`) — its `mod.rs` and `types.rs` carry the marker comment "Legacy … retained after QuickJS runtime removal." Do not assume the runtime can execute a `.skill` package end-to-end; check what `ops_install` and the current agent tool path actually do before planning a feature that needs it.

### Controller migration checklist

- `src/openhuman/<domain>/mod.rs`: keep export-focused, add `mod schemas;` and re-export:
  - `all_controller_schemas as all_<domain>_controller_schemas`
  - `all_registered_controllers as all_<domain>_registered_controllers`
- `src/openhuman/<domain>/schemas.rs` must define:
  - `schemas(function: &str) -> ControllerSchema`
  - `all_controller_schemas() -> Vec<ControllerSchema>`
  - `all_registered_controllers() -> Vec<RegisteredController>`
  - domain handler fns `fn handle_*(_: Map<String, Value>) -> ControllerFuture`
- Handlers should delegate to existing domain `rpc.rs` functions during migration.
- Wire domain exports into `src/core/all.rs` for both declared schemas and registered handlers.
- Keep adapters generic: do not add domain-specific logic to `src/core/cli.rs` or `src/core/jsonrpc.rs`.
- Remove migrated method branches from `src/rpc/dispatch.rs` once registry coverage is in place.

### Event bus (`src/core/event_bus/`)

A typed pub/sub event bus for **decoupled cross-module communication** plus a **native, in-process typed request/response** surface. Both are singletons — one instance each for the whole application. Do **not** construct `EventBus` or `NativeRegistry` directly; use the module-level functions.

**When to use which surface:**

- **Broadcast events** (`publish_global` / `subscribe_global`) — fire-and-forget notification. One publisher, many subscribers, no return value. Use when a module needs to _announce_ something happened and other modules may react independently.
- **Native request/response** (`register_native_global` / `request_native_global`) — one-to-one typed Rust dispatch keyed by a method string. **Zero serialization**: trait objects (`Arc<dyn Provider>`), streaming channels (`mpsc::Sender<T>`), oneshot senders, and anything else `Send + 'static` all pass through unchanged. Use when a module needs a typed return value from another module in-process. This is **internal-only** — anything that needs to be callable over JSON-RPC should register against `src/core/all.rs` instead.

**Core types** (all in `src/core/event_bus/`):

| Type | File | Purpose |
|------|------|---------|
| `DomainEvent` | `events.rs` | `#[non_exhaustive]` enum — all cross-module events live here, grouped by domain |
| `EventBus` | `bus.rs` | Singleton backed by `tokio::sync::broadcast`. Construction is `pub(crate)` — tests only |
| `NativeRegistry` / `NativeRequestError` | `native_request.rs` | In-process typed request/response registry keyed by method name. Rust types only — passes trait objects, `mpsc::Sender`, and `oneshot::Sender` through without serialization |
| `EventHandler` | `subscriber.rs` | Async trait with optional `domains()` filter for selective subscription |
| `SubscriptionHandle` | `subscriber.rs` | RAII handle — subscriber task is cancelled on drop |
| `TracingSubscriber` | `tracing.rs` | Built-in debug logger for all events (registered at startup) |

**Singleton API** (all modules use these — never hold or pass `EventBus` / `NativeRegistry` instances):

| Function | Purpose |
|----------|---------|
| `event_bus::init_global(capacity)` | Initialize both singletons (broadcast bus + native registry) at startup (once) |
| `event_bus::publish_global(event)` | Publish a broadcast event from anywhere (no-op if not yet initialized) |
| `event_bus::subscribe_global(handler)` | Subscribe to broadcast events from anywhere (returns `None` if not yet initialized) |
| `event_bus::register_native_global(method, handler)` | Register a typed native request handler for a method name — called at startup by each domain's `bus.rs` |
| `event_bus::request_native_global(method, req)` | Dispatch a typed native request to the registered handler — zero serialization |
| `event_bus::global()` / `event_bus::native_registry()` | Get the underlying singleton for advanced use |

**Domains:** `agent`, `memory`, `channel`, `cron`, `skill`, `tool`, `webhook`, `system`. See `events.rs` for the full variant list — events carry rich payloads so subscribers have everything they need.

**Domain subscriber files** — each domain owns its `bus.rs` with `EventHandler` impls:
- `cron/bus.rs` — `CronDeliverySubscriber` (delivers job output to channels)
- `webhooks/bus.rs` — `WebhookRequestSubscriber` (routes incoming requests to skills, emits responses via socket)
- `channels/bus.rs` — `ChannelInboundSubscriber` (runs agent loop for inbound socket messages)
- `skills/bus.rs` — stub for future subscribers

**Adding events for a new domain:**

1. Add variants to `DomainEvent` in `events.rs` (prefix with domain name, e.g. `BillingInvoiceCreated { ... }`).
2. Add the domain string to the `domain()` match arm.
3. Create a `bus.rs` file **inside your domain module** (e.g. `src/openhuman/billing/bus.rs`) for subscriber implementations — each domain owns its handlers.
4. Register subscribers in startup (e.g. `channels/runtime/startup.rs`) via the singleton.
5. Publish events with `event_bus::publish_global(DomainEvent::YourEvent { ... })`.

**Example — publishing:**
```rust
use crate::core::event_bus::{publish_global, DomainEvent};

publish_global(DomainEvent::CronDeliveryRequested {
    job_id: job.id.clone(),
    channel: "telegram".into(),
    target: "chat-123".into(),
    output: "Job completed".into(),
});
```

**Example — subscribing (trait-based, in `<domain>/bus.rs`):**
```rust
use crate::core::event_bus::{DomainEvent, EventHandler};
use async_trait::async_trait;

pub struct MyDomainSubscriber { /* dependencies */ }

#[async_trait]
impl EventHandler for MyDomainSubscriber {
    fn name(&self) -> &str { "my_domain::handler" }
    fn domains(&self) -> Option<&[&str]> { Some(&["cron"]) } // filter by domain
    async fn handle(&self, event: &DomainEvent) {
        if let DomainEvent::CronJobCompleted { job_id, success } = event {
            // react to the event
        }
    }
}
```

**Convention:** Name the handler struct `<Purpose>Subscriber` (e.g. `CronDeliverySubscriber`) and the `name()` return value `"<domain>::<purpose>"` for grep-friendly tracing output.

**Adding a native request handler for a new domain:**

1. Define the **request and response types** in the domain (e.g. `src/openhuman/billing/bus.rs`). Use owned fields, `Arc`s, and channels — not borrows. Types only need `Send + 'static`, not `Serialize`.
2. Register the handler at startup from the same `bus.rs`, keyed by a stable method name prefixed with the domain (e.g. `"billing.charge_invoice"`).
3. Callers import the request/response types from the domain's public surface and dispatch via `request_native_global`.
4. Method name convention: `"<domain>.<verb>"` — same naming scheme as JSON-RPC method roots for consistency, but these are **not** exposed over JSON-RPC.

**Example — native request (typed request/response, in `<domain>/bus.rs`):**
```rust
use crate::core::event_bus::{register_native_global, request_native_global};
use std::sync::Arc;
use tokio::sync::mpsc;

// Request carries non-serializable state directly — trait objects and
// streaming channels all pass through unchanged.
pub struct BillingChargeRequest {
    pub provider: Arc<dyn BillingProvider>,
    pub amount_cents: u64,
    pub progress_tx: Option<mpsc::Sender<String>>,
}
pub struct BillingChargeResponse {
    pub charge_id: String,
}

// At startup:
pub async fn register_billing_handlers() {
    register_native_global::<BillingChargeRequest, BillingChargeResponse, _, _>(
        "billing.charge",
        |req| async move {
            let id = req.provider.charge(req.amount_cents).await
                .map_err(|e| e.to_string())?;
            Ok(BillingChargeResponse { charge_id: id })
        },
    ).await;
}

// From another module:
let resp: BillingChargeResponse = request_native_global(
    "billing.charge",
    BillingChargeRequest { provider, amount_cents: 500, progress_tx: None },
).await?;
```

**Tests:** override production handlers by calling `register_native_global` again for the same method before exercising the code under test — the most recent registration wins. For full isolation, construct a fresh `NativeRegistry` directly via `NativeRegistry::new()` and use its `register` / `request` methods.

---

## App theme & design system

**Design intent**: Premium, calm visual language — ocean primary (`#4A83DD`), sage / amber / coral semantic colors, Inter + Cabinet Grotesk + JetBrains Mono, Tailwind with custom radii/spacing/shadows. Implementation tokens live in [`app/tailwind.config.js`](app/tailwind.config.js).

## Desktop shell (Tauri) vs application code

In the parent **OpenHuman** desktop app, **Tauri / Rust is a delivery vehicle**: windowing, process lifecycle, IPC to the core sidecar, and other host concerns. **Keep as much UI behavior and product logic as practical in TypeScript/React** (`app/`). Avoid growing Rust in the shell for flows that belong in the web layer unless there is a hard platform or security reason.

## Git workflow

- **Never write code on `main`.** Before making any code changes, fork a new branch off the latest `main` (`git fetch upstream && git checkout -b <branch> upstream/main`). All work happens on that feature branch; `main` stays clean and only advances via merged PRs.
- **GitHub issues on upstream** — File and track issues on **[tinyhumansai/openhuman](https://github.com/tinyhumansai/openhuman/)** ([Issues](https://github.com/tinyhumansai/openhuman/issues)), not only a fork’s tracker, unless the workflow explicitly says otherwise.
- **GitHub issue templates** — Use **[`.github/ISSUE_TEMPLATE/feature.md`](.github/ISSUE_TEMPLATE/feature.md)** for new features and **[`.github/ISSUE_TEMPLATE/bug.md`](.github/ISSUE_TEMPLATE/bug.md)** for bugs; keep the same section structure and fill every required part. AI-authored issues should follow those templates verbatim.
- **Open pull requests on upstream** — Always create PRs against **[tinyhumansai/openhuman](https://github.com/tinyhumansai/openhuman)** ([pull requests](https://github.com/tinyhumansai/openhuman/pulls)), not only a fork’s default remote, unless the workflow explicitly says otherwise.
- **Public repo**; push to your working branch; PRs target **`main`**.
- **Agent branch rule** — If an agent starts work while checked out on `main`, it should create its own descriptive working branch before committing or pushing. Do not leave agent-authored commits on local `main`; move the pending work onto the new branch and ship from there.
- Use [`.github/PULL_REQUEST_TEMPLATE.md`](.github/PULL_REQUEST_TEMPLATE.md); AI-generated PR text should follow its sections and checklist.

---

## Coding philosophy

- **Unix-style modules**: Prefer **individual modules** with a **single, sharp responsibility**—each should do one thing really well. Compose behavior through small, well-named units and clear boundaries instead of monolithic code.
- **Tests before the next layer**: Ship **enough unit tests and coverage** for the behavior you are adding or changing **before** building additional features on top of it. Treat untested code as incomplete; do not accumulate depth on a shaky base.
- **Documentation with code**: New or changed behavior must ship with matching documentation. At minimum, add concise rustdoc / code comments where the flow is not obvious, and update `AGENTS.md`, architecture docs, or feature docs when repository rules or user-visible behavior change.

---

## Debug logging rule (must follow)

- **Default to verbose diagnostics on new/changed flows**: Add substantial, development-oriented logs while implementing features or fixes so issues are easy to trace end-to-end.
- **Log critical checkpoints**: Include logs at entry/exit points, branch decisions, external calls, retries/timeouts, state transitions, and error handling paths.
- **Use structured, grep-friendly context**: Prefer stable prefixes (for example `[domain]`, `[rpc]`, `[ui-flow]`) and include correlation fields such as request IDs, method names, and entity IDs when available.
- **Platform conventions**: In Rust, use `log` / `tracing` at `debug` or `trace`; in `app/`, use namespaced `debug` logs and dev-only detail as needed.
- **Keep logs safe**: Never log secrets or sensitive payloads (API keys, JWTs, credentials, full PII). Redact or omit sensitive fields.
- **Treat debuggability as a deliverable**: Changes lacking sufficient logging for diagnosis are incomplete and should be updated before handoff.

---

## Feature design workflow (new capabilities)

Follow this order so behavior is **specified**, **proven in Rust**, **proven over RPC**, then **surfaced in the UI** with matching tests.

1. **Specify against the current codebase** — Ground the design in **existing** domains, controller/registry patterns, and JSON-RPC naming (`openhuman.<namespace>_<function>`). Reuse or extend documented flows in [`gitbooks/developing/architecture.md`](gitbooks/developing/architecture.md) and sibling guides; avoid parallel architectures.
2. **Implement in Rust** — Add domain logic under `src/openhuman/<domain>/`, wire **schemas + registered handlers** into the shared registry, and land **unit tests** in the crate (`cargo test -p openhuman`, focused modules) until the feature is correct in isolation.
3. **JSON-RPC E2E** — Add or extend **integration-style tests** that call the real HTTP JSON-RPC surface (e.g. [`tests/json_rpc_e2e.rs`](tests/json_rpc_e2e.rs), mock backend / [`scripts/test-rust-with-mock.sh`](scripts/test-rust-with-mock.sh) as appropriate) so methods, params, and outcomes match what the UI will call.
4. **UI in the Tauri app** — Build **React** screens, state, and **`core_rpc_relay` / `coreRpcClient`** usage in `app/`; keep **business rules** in the core, not duplicated in the shell.
5. **App unit tests** — Cover components, hooks, and clients with **Vitest** (`pnpm test` / `pnpm test:unit` in `app/`).
6. **App E2E** — Add **desktop E2E** specs where the feature is user-visible (`pnpm test:e2e*`, isolated workspace — see [Testing Guide (Unit + E2E)](#testing-guide-unit--e2e)) so the full stack (UI → Tauri → sidecar) behaves as intended.

**Capability catalog** — When a change adds, removes, renames, relocates, or materially changes a user-facing feature, update **`src/openhuman/about_app/`** in the same work so the runtime capability catalog remains the source of truth for what the app can do.

**Debug logging (throughout)** — Add **lots of development-oriented logging** as you build, not as an afterthought. In **Rust**, use `log` / `tracing` at **`debug`** or **`trace`** on RPC entry and exit, error paths, state transitions, and any branch that is hard to infer from tests alone. In **`app/`**, follow existing patterns (e.g. the **`debug`** npm package with a **namespace** per area) plus **dev-only** detail where useful. Prefer **grep-friendly prefixes** (`[feature]`, domain name, or JSON-RPC method) so terminal output from **sidecar**, **Tauri**, and **WebView** can be correlated during `pnpm dev` / `tauri dev`. **Never** log secrets, raw JWTs, API keys, or full PII—redact or omit.

**Planning rule:** When scoping a feature, define the **E2E scenarios (core RPC + app)** up front. Those scenarios should **cover the full intended scope**—happy paths, failure modes, auth or policy gates, and regressions you care about. If a scenario is not testable end-to-end, the spec is incomplete or the cut is too large; split or add harness support first.

---

## Key patterns (concise)

- **Debug logging**: Ship **heavy `debug`/`trace` (Rust)** and **namespaced `debug` / dev logs (`app/`)** on new flows so sidecar + WebView output is easy to grep; see [Feature design workflow](#feature-design-workflow-new-capabilities). Never log secrets or raw tokens.
- **`src/openhuman/`**: New features go in a **folder/module**, not new root-level `src/openhuman/*.rs` files (see Rust core section).
- **File size**: Prefer ≤ ~500 lines per source file; split modules when growing.
- **Pre-merge checks** (when touching code): Prettier, ESLint, `tsc --noEmit` in `app/`; `cargo fmt` + `cargo check` for changed Rust (`Cargo.toml` at root and/or `app/src-tauri/Cargo.toml` as appropriate).
- **No dynamic imports** in production **`app/src`** code — use **static** `import` / `import type` at the top of the module. Do **not** use `import()` (async dynamic import), `React.lazy(() => import(...))`, or `await import('…')` to load app modules, Tauri APIs, or RPC clients. **Why:** predictable chunk graph, simpler static analysis, fewer surprises in Tauri + Vite, and easier code review. **If a module must not run at load time** (e.g. heavy optional path), use a static import and **guard the call site** with `try/catch` or an explicit runtime check instead of deferring module load via dynamic import. **Exceptions:** Vitest harness patterns (`vi.importActual`, dynamic imports **only** inside `*.test.ts` / `__tests__` / `test/setup.ts` when required by the runner); ambient `typeof import('…')` in `.d.ts`; config files (e.g. `tailwind.config.js` JSDoc).- **Type-only imports**: `import type` where appropriate.
- **Dual socket / tool sync**: If you change realtime protocol, keep **frontend** (`socketService` / MCP transport) and **core** socket behavior aligned (see [`gitbooks/developing/architecture.md`](gitbooks/developing/architecture.md) dual-socket section).
- **i18n for all UI text**: Every user-visible string in `app/src/**` (headings, labels, button text, placeholders, status chips, toasts, dialog copy, `aria-label`, etc.) must go through `useT()` from `app/src/lib/i18n/I18nContext`. Hard-coded literals in JSX or `label=`/`placeholder=`/`aria-label=` props are not allowed. Add the new key to [`app/src/lib/i18n/en.ts`](app/src/lib/i18n/en.ts) in the same PR — other locales fall back to English. **Exceptions:** developer-only debug logs, code identifiers, and non-display data (URLs, slugs, technical sentinel values).
- **i18n chunk files — update ALL locales**: The source-of-truth translation files are the **chunk files** under `app/src/lib/i18n/chunks/` (`en-{1..5}.ts` plus `<locale>-{1..5}.ts` for each locale). When adding or changing keys in `en.ts`, you **must also** add them to the corresponding English chunk file (`en-N.ts`) **and** to the same chunk number for every non-English locale (use the English value as a placeholder — translators fill in later). CI enforces parity via `pnpm i18n:check`; missing keys in any locale chunk will fail the i18n coverage gate. Locales: `ar`, `bn`, `de`, `es`, `fr`, `hi`, `id`, `it`, `ko`, `pt`, `ru`, `zh-CN`.

---

## Platform notes

- **macOS deep links**: Often require a built **`.app`** bundle; not only `tauri dev`.
- **`window.__TAURI__`**: Not assumed at module load; use `isTauri()` (from `app/src/services/webviewAccountService.ts`) or wrap `invoke(...)` in `try/catch`.
- **Core is in-process**: `core_rpc` reaches `http://127.0.0.1:<port>/rpc` (default port `7788`) authenticated with `OPENHUMAN_CORE_TOKEN`. `scripts/stage-core-sidecar.mjs` no longer exists; `pnpm core:stage` is a no-op echo (sidecar removed in PR #1061). For standalone debugging: `./target/debug/openhuman-core serve` writes its token to `{workspace}/core.token` (default `~/.openhuman-staging/core.token` under `OPENHUMAN_APP_ENV=staging`); public endpoints `GET /health`, `GET /schema`, `GET /events` need no auth.

---

_Last aligned with monorepo layout (`app/` + root `src/`), in-process core (no sidecar), QuickJS removed, skills catalog on GitHub (`tinyhumansai/openhuman-skills`), and Tauri shell IPC as of `openhuman-app` v0.53.45 / repo `main`._

---

## Cursor Cloud specific instructions

### Environment overview

Two services run independently for development:

| Service | Start command | Port | Notes |
|---------|--------------|------|-------|
| **Vite dev server** | `pnpm dev` (from repo root) | 1420 | React frontend with HMR |
| **Core JSON-RPC server** | `./target/debug/openhuman-core serve` | 7788 | Rust core, writes bearer token to `~/.openhuman-staging/core.token` |

The app connects to a **remote staging backend** at `https://staging-api.tinyhumans.ai` — there is no local backend to run.

### Running the core server standalone

The core generates a bearer token at startup written to `{workspace_dir}/core.token` (default `~/.openhuman-staging/core.token` when `OPENHUMAN_APP_ENV=staging`). Read that file for authenticated RPC calls:

```bash
TOKEN=$(cat ~/.openhuman-staging/core.token)
curl http://localhost:7788/rpc -X POST \
  -H 'Content-Type: application/json' \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"jsonrpc":"2.0","method":"core.ping","params":{},"id":1}'
```

Public endpoints (no token needed): `GET /health`, `GET /schema`, `GET /events`.

### Linux build dependencies (non-obvious)

Compiling the Rust core on Linux requires these system packages beyond the basics:
`libasound2-dev libxi-dev libxtst-dev libxdo-dev libudev-dev libssl-dev clang cmake pkg-config libstdc++-14-dev`

The `libstdc++-14-dev` package is needed because clang selects GCC 14 headers; without it, whisper-rs-sys fails with `fatal error: 'array' file not found`. A symlink may also be needed: `ln -sf /usr/lib/gcc/x86_64-linux-gnu/13/libstdc++.so /usr/lib/x86_64-linux-gnu/libstdc++.so`.

### Quick reference for common dev commands

All commands are documented in `CLAUDE.md` and `AGENTS.md` above. The most-used subset:

- **Lint**: `pnpm lint` (ESLint, 0 errors expected; warnings are acceptable)
- **Typecheck**: `pnpm typecheck` (`tsc --noEmit`)
- **Unit tests**: `pnpm test` (Vitest, runs 1000+ tests)
- **Rust check**: `cargo check --manifest-path Cargo.toml`
- **Rust tests**: `cargo test --lib` (5600+ tests)
- **Format check**: `pnpm format:check`

### Running the Tauri desktop app on Linux cloud VMs

The full desktop app can be built and run on headless Linux VMs with:

```bash
export CEF_PATH="$HOME/Library/Caches/tauri-cef"
export LD_LIBRARY_PATH="$CEF_PATH/146.0.9/cef_linux_x86_64:$LD_LIBRARY_PATH"
source scripts/load-dotenv.sh
cargo tauri dev -- -- --no-sandbox
```

Key requirements:
- `--no-sandbox` is required because Chromium refuses to run as root without it.
- `LD_LIBRARY_PATH` must include the CEF distribution directory so `libcef.so` is found at runtime.
- The vendored CEF-aware `cargo-tauri` must be installed first via `bash scripts/ensure-tauri-cli.sh`.
- First build downloads ~300MB CEF binary and compiles ~900 crates; subsequent builds are incremental.
- GTK/cairo libraries are required: `libgtk-3-dev libwebkit2gtk-4.1-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev libglib2.0-dev libcairo2-dev libpango1.0-dev libgdk-pixbuf-2.0-dev libatk1.0-dev libdbus-1-dev`.
- WebGL errors in the log (`ContextResult::kFatalFailure: WebGL1/2 blocklisted`) are normal on GPU-less VMs and do not affect app functionality.

### Gotchas

- `pnpm install` may warn about ignored build scripts (`@sentry/cli`, `esbuild`, etc.). The esbuild binary is correctly installed via its native platform package despite the warning — Vite and Vitest work fine.
- Git submodules (`app/src-tauri/vendor/tauri-cef`, `app/src-tauri/vendor/tauri-plugin-notification`) must be initialized for Tauri shell compilation. Run `git submodule update --init --recursive` if not already done.
- `pnpm test:unit` does not exist at the root level; use `pnpm test` instead (which delegates to `vitest run` in the `app` workspace).
- The Tauri shell `cargo check` requires GTK/desktop system libraries; without them, the pre-push hook's `pnpm rust:check` will fail. Use `--no-verify` on push if GTK libs are missing and the change is unrelated to the Tauri shell.


<claude-mem-context>
# Memory Context

# [openhuman] recent context, 2026-04-22 9:52am PDT

Legend: 🎯session 🔴bugfix 🟣feature 🔄refactor ✅change 🔵discovery ⚖️decision
Format: ID TIME TYPE TITLE
Fetch details: get_observations([IDs]) | Search: mem-search skill

Stats: 20 obs (8,333t read) | 593,112t work | 99% savings

### Apr 22, 2026
2848 9:07a ✅ openhuman: All Three Review Branches Pushed to Fork Successfully
2849 " 🔵 openhuman review-daemon-lifecycle: Two Post-Push Issues — Unstaged Prettier Changes + Missing tauri-cef Vendor
2851 9:08a ✅ openhuman daemon lifecycle: Prettier Format Committed as Follow-Up
2855 9:09a ✅ openhuman: All Three Review Branches Fully Pushed — PRs Ready to Open
2857 9:10a 🔵 openhuman: GitHub Connector Cannot Create PRs to tinyhumansai/openhuman — 403 Forbidden
2858 9:11a 🔵 openhuman webhooks-ingress: Session Stalled — Instruction Not Processed After 10+ Minutes
2860 " 🔵 openhuman webhooks: WebhooksDebugPanel Architecture for E2E Smoke Spec
2861 9:13a 🔵 openhuman webhooks-ingress: Full Spec Surface Mapped — RPC Log Strings + UI Navigation Path
2866 9:15a 🟣 openhuman webhooks-ingress: webhooks-ingress-flow.spec.ts Written
2869 9:18a ⚖️ openhuman Memory Refactor Plan: Trait Shape, L1 Pointer, and Missing Pieces
2871 " 🔵 openhuman Memory Architecture: Auto-Inject Pattern Has 3 Separate Implementations
2873 9:31a 🟣 openhuman: Draft PR Opened — Config Runtime Dir Refactor for Testability
2874 9:32a 🟣 openhuman: 3 More Draft PRs Opened — Threads Schema, Daemon Lifecycle, Webhooks E2E
2875 9:33a 🔵 openhuman Memory Namespace: 3 Auto-Inject Sites, Not 1
2876 " ⚖️ openhuman Memory Refactor: Breaking Trait Change + Flag-Off + ToolDiscovery Hybrid
2877 " ✅ Memory Namespace Refactor Plan Written to docs/plans/memory-namespace-refactor.md
2879 9:34a 🔵 openhuman Memory Trait: 15 Impls, Not 14; MemoryRecalled Has No Live Emit Site
2880 " 🔵 openhuman SQLite Schema: memory_docs Already Has namespace Column; Migration Scope Minimal
2881 " 🔵 openhuman Memory Trait Current Signatures: No Namespace Param on Any Method
2882 " 🔵 openhuman Eval Infra: Does Not Exist; Phase D Requires Bootstrap from Scratch

Access 593k tokens of past work via get_observations([IDs]) or mem-search skill.
</claude-mem-context>
