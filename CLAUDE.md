# OpenHuman

**AI-powered assistant for communities — React + Tauri v2 desktop app with a Rust core (JSON-RPC / CLI) and sandboxed QuickJS skills.**

This file orients contributors and coding agents. Authoritative narrative architecture: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). Frontend layout: [`docs/src/README.md`](docs/src/README.md). Tauri shell: [`docs/src-tauri/README.md`](docs/src-tauri/README.md).

---

## Repository layout

| Path                    | Role                                                                                                                                                                                                        |
| ----------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **`app/`**              | Yarn workspace **`openhuman-app`**: Vite + React (`app/src/`), Tauri desktop host (`app/src-tauri/`), Vitest tests                                                                                          |
| **Repo root `src/`**    | Rust library **`openhuman_core`** and **`openhuman`** CLI binary entrypoint (`src/main.rs`) — `core_server`, `openhuman::*` domains, skills runtime (QuickJS / `rquickjs`), MCP routing in the core process |
| **Skills registry**     | **[`tinyhumansai/openhuman-skills`](https://github.com/tinyhumansai/openhuman-skills)** on GitHub — canonical skill packages and TS build; not vendored in this tree (see blurb below).                     |
| **`Cargo.toml`** (root) | Core crate; `cargo build --bin openhuman` produces the sidecar the UI stages via `app`’s `core:stage`                                                                                                       |
| **`docs/`**             | Architecture and module guides (numbered pages under `docs/src/`, `docs/src-tauri/`)                                                                                                                        |

Commands in documentation assume the **repo root** unless noted: `yarn dev` runs the `app` workspace.

**Skills registry:** Skill sources and the bundler live in **[github.com/tinyhumansai/openhuman-skills](https://github.com/tinyhumansai/openhuman-skills)**. Clone that repository to author or change skills (`yarn install`, `yarn build`). The desktop app’s skills catalog defaults to that GitHub slug; override with `VITE_SKILLS_GITHUB_REPO` (see [`app/src/utils/config.ts`](app/src/utils/config.ts)).

---

## Runtime scope

- **Shipped product**: desktop — Windows, macOS, Linux (see [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) “Platform reach”).
- **Tauri host** (`app/src-tauri`): **desktop-only** (`compile_error!` for non-desktop targets). Do not add Android/iOS branches inside `app/src-tauri`.
- **Core binary** (`openhuman`): spawned/staged as a **sidecar**; the Web UI talks to it over HTTP (`core_rpc_relay` + `core_rpc` client), not by re-implementing domain logic in the shell.

**Where logic lives**

- **Rust (`openhuman` / repo root `src/`)**: **Business logic and execution**—domains, skills runtime, RPC, persistence, and CLI behavior. This is the authoritative place for rules and side effects.
- **Tauri + React (`app/`)**: **Interaction and UX**—screens, navigation, input, accessibility, windowing, and bridging to the core. The shell presents and orchestrates; it does not duplicate core business rules.

---

## Commands (from repository root)

```bash
# Frontend + Tauri dev (workspace delegates to app/)
yarn dev

# Desktop with Tauri (loads env via scripts/load-dotenv.sh)
yarn tauri dev

# Production UI build (app workspace)
yarn build

# Typecheck / lint / format (app workspace)
yarn typecheck
yarn lint
yarn format
yarn format:check

# Stage openhuman core binary next to Tauri resources (required for core RPC)
cd app && yarn core:stage

# Skills — develop in the GitHub registry repo, then build (see tinyhumansai/openhuman-skills).
# If you keep a local clone path wired in app scripts, you can also run:
yarn workspace openhuman-app skills:build
yarn workspace openhuman-app skills:watch

# Rust — core library + CLI (repo root)
cargo check --manifest-path Cargo.toml
cargo build --manifest-path Cargo.toml --bin openhuman

# Rust — Tauri shell only
cargo check --manifest-path app/src-tauri/Cargo.toml
```

**Tests**: Vitest in `app/` (`yarn test`, `yarn test:coverage`). Rust tests via `cargo test` at repo root as wired in `app/package.json`.

**Quality**: ESLint + Prettier + Husky in the `app` workspace.

---

## Configuration

Environment variables are documented in two `.env.example` files:

- **[`.env.example`](.env.example)** (repo root) — Rust core, Tauri shell, backend URL, logging, proxy, storage, web search, local AI binary overrides. Loaded via `source scripts/load-dotenv.sh`.
- **[`app/.env.example`](app/.env.example)** — Frontend `VITE_*` vars (core RPC URL, backend URL, Sentry DSN, skills repo, dev helpers). Copy to `app/.env.local` for local overrides.

**Frontend config** is centralized in [`app/src/utils/config.ts`](app/src/utils/config.ts). All `VITE_*` env vars should be read there and re-exported — do not read `import.meta.env` directly in other files.

**Rust config** uses a TOML-based `Config` struct (`src/openhuman/config/schema/types.rs`) with env var overrides applied in `src/openhuman/config/schema/load.rs`. Env vars override config file values at runtime (e.g. `OPENHUMAN_API_KEY` overrides `config.api_key`).

---

## Testing Guide (Unit + E2E)

### Unit tests (Vitest)

- **Where tests live**: co-locate as `*.test.ts` / `*.test.tsx` under `app/src/**`.
- **Runner/config**: Vitest with `app/test/vitest.config.ts` and shared setup in `app/src/test/setup.ts`.
- **Run**:

```bash
yarn test:unit
yarn test:coverage
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
yarn mock:api
curl -s http://127.0.0.1:18473/__admin/health
```

### E2E tests (WDIO — dual platform)

Full guide: [`docs/E2E-TESTING.md`](docs/E2E-TESTING.md).

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
yarn test:e2e:build

# Run one spec
bash app/scripts/e2e-run-spec.sh test/e2e/specs/smoke.spec.ts smoke

# Run all flow specs
yarn test:e2e:all:flows

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
yarn test:e2e:build
bash app/scripts/e2e-run-spec.sh test/e2e/specs/smoke.spec.ts smoke
rm -rf "$OPENHUMAN_WORKSPACE"
```

- `OPENHUMAN_WORKSPACE` redirects core config + workspace storage away from `~/.openhuman`.
- Default reset strategy:
  - Rebuild/stage sidecar once per E2E run (`yarn test:e2e:build`).
  - Isolate state per test case with a fresh temp workspace (default behavior in `e2e-run-spec.sh`).

### Rust tests with mock backend

Use the shared mock backend runner so Rust unit/integration tests get deterministic API behavior:

```bash
yarn test:rust
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

### Test authoring checklist

- Add/update unit tests for logic changes before stacking additional features.
- Add/update E2E coverage for user-visible flows and cross-process integration behavior.
- Keep new tests independent, deterministic, and debuggable from logs alone.
- When touching core/sidecar behavior, validate both:
  - `yarn test:unit`
  - targeted E2E spec(s) via `app/scripts/e2e-run-spec.sh`

---

## Frontend (`app/src/`)

### Provider chain (`app/src/App.tsx`)

Order matters for auth and realtime:

`Redux Provider` → `PersistGate` → **`UserProvider`** → **`SocketProvider`** → **`AIProvider`** → **`SkillProvider`** → **`HashRouter`** → `AppRoutes`.

There is **no** `TelegramProvider` in the current tree; Telegram may appear in UI copy or legacy settings, but MTProto is not an active provider here.

### State (`app/src/store/`)

Redux Toolkit slices include **auth**, **user**, **socket**, **ai**, **skills**, **team**, and related modules. Prefer Redux (and persist where configured) over ad hoc `localStorage` for app state; see project rules for exceptions.

### Services (`app/src/services/`)

Singleton-style modules include **`apiClient`**, **`socketService`**, **`coreRpcClient`** (HTTP bridge to the core process), and domain **`api/*`** clients. There is **no** `mtprotoService` in this tree.

### MCP (`app/src/lib/mcp/`)

Transport, validation, and types for JSON-RPC-style messaging over Socket.io — **not** a large Telegram tool pack. Tooling for agents is driven by the **skills** system and backend; see `agentToolRegistry.ts` and core RPC.

### Routing (`app/src/AppRoutes.tsx`)

Hash routes include `/`, `/onboarding`, `/mnemonic`, `/home`, `/intelligence`, `/skills`, `/conversations`, `/invites`, `/agents`, `/settings/*`, plus `DefaultRedirect`. **No** dedicated `/login` route in `AppRoutes` (auth flows use the welcome/onboarding paths).

### AI configuration

Bundled prompts live under **`src/openhuman/agent/prompts/`** at the **repository root** (also bundled via `app/src-tauri/tauri.conf.json` `resources`). Loaders under `app/src/lib/ai/` use `?raw` imports, optional remote fetch, and in Tauri **`ai_get_config` / `ai_refresh_config`** for packaged content.

---

## Tauri shell (`app/src-tauri/`)

Thin desktop host: window management, daemon health bridging, **core process lifecycle** (`core_process`, `CoreProcessHandle`), and **JSON-RPC relay** to the **`openhuman`** sidecar (`core_rpc_relay`, `core_rpc`).

Registered IPC commands (see [`docs/src-tauri/02-commands.md`](docs/src-tauri/02-commands.md)) include **`greet`**, **`write_ai_config_file`**, **`ai_get_config`**, **`ai_refresh_config`**, **`core_rpc_relay`**, **window** commands, and **OpenHuman service / daemon host** helpers (`openhuman_*`).

Deep link plugin is registered where supported; behavior is platform-specific (see platform notes below).

---

## Rust core (repo root `src/`)

- **`openhuman/`** — Domain logic (skills, memory, channels, config, …). RPC controllers live in **`rpc.rs`** files per domain; use **`RpcOutcome<T>`** pattern per [`AGENTS.md`](AGENTS.md) / internal rules.
- **`src/openhuman/` module layout**: **New** functionality must live in a **dedicated subdirectory** (its own folder/module, e.g. `openhuman/my_domain/mod.rs` plus related files, or a new subfolder under an existing domain). Do **not** add new standalone `*.rs` files directly at `src/openhuman/` root; place new code in a module directory and declare it from `mod.rs` (or merge into an existing domain folder).
- **Controller schema contract**: Shared controller metadata types live in **`src/core/mod.rs`** (`ControllerSchema`, `FieldSchema`, `TypeSchema`) and are consumed by adapters (RPC/CLI) in different ways.
- **Domain schema files**: For each domain, define controller schema metadata in a dedicated module inside the domain folder (example: **`src/openhuman/cron/schemas.rs`**) and export from the domain `mod.rs`.
- **Controller-only exposure rule**: Expose domain functionality to **CLI and JSON-RPC through the controller registry** (`schemas.rs` + registered handlers). Do **not** add domain-specific branches or one-off transport logic in `src/core/cli.rs` or `src/core/jsonrpc.rs` just to expose a feature.
- **Light `mod.rs` rule**: Keep domain `mod.rs` files light and export-focused. Put operational code in sibling files (example: `ops.rs`, `store.rs`, `schedule.rs`, `types.rs`), then re-export the public API from `mod.rs`.
- **`core_server/`** — Transport only: Axum/HTTP, JSON-RPC envelope, CLI parsing, **dispatch** (`core_server::dispatch`) — **no** heavy business logic here.
- **Layering**: Implementation in `openhuman::<domain>/`, controllers in `openhuman::<domain>/rpc.rs`, routes in `core_server/`.

Skills runtime uses **QuickJS** (`rquickjs`) in **`src/openhuman/skills/`** (e.g. `qjs_skill_instance.rs`, `qjs_engine.rs`), not V8/deno_core in this repository.

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

### Event bus (`src/openhuman/event_bus/`)

A typed pub/sub event bus for **decoupled cross-module communication**. The bus is a **singleton** — one instance handles all events for the entire application. Do **not** construct `EventBus` directly; use the module-level functions.

**When to use the event bus:** Use events when a module needs to _notify_ other modules of something that happened (fire-and-forget). Do **not** use events for request/response flows where the caller needs a return value — use direct function calls or RPC for those.

**Core types** (all in `src/openhuman/event_bus/`):

| Type | File | Purpose |
|------|------|---------|
| `DomainEvent` | `events.rs` | `#[non_exhaustive]` enum — all cross-module events live here, grouped by domain |
| `EventBus` | `bus.rs` | Singleton backed by `tokio::sync::broadcast`. Construction is `pub(crate)` — tests only |
| `EventHandler` | `subscriber.rs` | Async trait with optional `domains()` filter for selective subscription |
| `SubscriptionHandle` | `subscriber.rs` | RAII handle — subscriber task is cancelled on drop |
| `TracingSubscriber` | `tracing.rs` | Built-in debug logger for all events (registered at startup) |

**Singleton API** (all modules use these — never hold or pass `EventBus` instances):

| Function | Purpose |
|----------|---------|
| `event_bus::init_global(capacity)` | Initialize the singleton at startup (once) |
| `event_bus::publish_global(event)` | Publish from anywhere (no-op if not yet initialized) |
| `event_bus::subscribe_global(handler)` | Subscribe from anywhere (returns `None` if not yet initialized) |
| `event_bus::global()` | Get `Option<&'static EventBus>` for advanced use |

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
use crate::openhuman::event_bus::{publish_global, DomainEvent};

publish_global(DomainEvent::CronDeliveryRequested {
    job_id: job.id.clone(),
    channel: "telegram".into(),
    target: "chat-123".into(),
    output: "Job completed".into(),
});
```

**Example — subscribing (trait-based, in `<domain>/bus.rs`):**
```rust
use crate::openhuman::event_bus::{DomainEvent, EventHandler};
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

---

## App theme & design system

**Design intent**: Premium, calm visual language — ocean primary (`#4A83DD`), sage / amber / coral semantic colors, Inter + Cabinet Grotesk + JetBrains Mono, Tailwind with custom radii/spacing/shadows. Details: [`docs/DESIGN_GUIDELINES.md`](docs/DESIGN_GUIDELINES.md).

## Desktop shell (Tauri) vs application code

In the parent **OpenHuman** desktop app, **Tauri / Rust is a delivery vehicle**: windowing, process lifecycle, IPC to the core sidecar, and other host concerns. **Keep as much UI behavior and product logic as practical in TypeScript/React** (`app/`). Avoid growing Rust in the shell for flows that belong in the web layer unless there is a hard platform or security reason.

## Git workflow

- **GitHub issues on upstream** — File and track issues on **[tinyhumansai/openhuman](https://github.com/tinyhumansai/openhuman/)** ([Issues](https://github.com/tinyhumansai/openhuman/issues)), not only a fork’s tracker, unless the workflow explicitly says otherwise.
- **GitHub issue templates** — Use **[`.github/ISSUE_TEMPLATE/feature.md`](.github/ISSUE_TEMPLATE/feature.md)** for new features and **[`.github/ISSUE_TEMPLATE/bug.md`](.github/ISSUE_TEMPLATE/bug.md)** for bugs; keep the same section structure and fill every required part. AI-authored issues should follow those templates verbatim.
- **Open pull requests on upstream** — Always create PRs against **[tinyhumansai/openhuman](https://github.com/tinyhumansai/openhuman)** ([pull requests](https://github.com/tinyhumansai/openhuman/pulls)), not only a fork’s default remote, unless the workflow explicitly says otherwise.
- **Public repo**; push to your working branch; PRs target **`main`**.
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

1. **Specify against the current codebase** — Ground the design in **existing** domains, controller/registry patterns, and JSON-RPC naming (`openhuman.<namespace>_<function>`). Reuse or extend documented flows in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) and sibling guides; avoid parallel architectures.
2. **Implement in Rust** — Add domain logic under `src/openhuman/<domain>/`, wire **schemas + registered handlers** into the shared registry, and land **unit tests** in the crate (`cargo test -p openhuman`, focused modules) until the feature is correct in isolation.
3. **JSON-RPC E2E** — Add or extend **integration-style tests** that call the real HTTP JSON-RPC surface (e.g. [`tests/json_rpc_e2e.rs`](tests/json_rpc_e2e.rs), mock backend / [`scripts/test-rust-with-mock.sh`](scripts/test-rust-with-mock.sh) as appropriate) so methods, params, and outcomes match what the UI will call.
4. **UI in the Tauri app** — Build **React** screens, state, and **`core_rpc_relay` / `coreRpcClient`** usage in `app/`; keep **business rules** in the core, not duplicated in the shell.
5. **App unit tests** — Cover components, hooks, and clients with **Vitest** (`yarn test` / `yarn test:unit` in `app/`).
6. **App E2E** — Add **desktop E2E** specs where the feature is user-visible (`yarn test:e2e*`, isolated workspace — see [Testing Guide (Unit + E2E)](#testing-guide-unit--e2e)) so the full stack (UI → Tauri → sidecar) behaves as intended.

**Capability catalog** — When a change adds, removes, renames, relocates, or materially changes a user-facing feature, update **`src/openhuman/about_app/`** in the same work so the runtime capability catalog remains the source of truth for what the app can do.

**Debug logging (throughout)** — Add **lots of development-oriented logging** as you build, not as an afterthought. In **Rust**, use `log` / `tracing` at **`debug`** or **`trace`** on RPC entry and exit, error paths, state transitions, and any branch that is hard to infer from tests alone. In **`app/`**, follow existing patterns (e.g. the **`debug`** npm package with a **namespace** per area) plus **dev-only** detail where useful. Prefer **grep-friendly prefixes** (`[feature]`, domain name, or JSON-RPC method) so terminal output from **sidecar**, **Tauri**, and **WebView** can be correlated during `yarn dev` / `tauri dev`. **Never** log secrets, raw JWTs, API keys, or full PII—redact or omit.

**Planning rule:** When scoping a feature, define the **E2E scenarios (core RPC + app)** up front. Those scenarios should **cover the full intended scope**—happy paths, failure modes, auth or policy gates, and regressions you care about. If a scenario is not testable end-to-end, the spec is incomplete or the cut is too large; split or add harness support first.

---

## Key patterns (concise)

- **Debug logging**: Ship **heavy `debug`/`trace` (Rust)** and **namespaced `debug` / dev logs (`app/`)** on new flows so sidecar + WebView output is easy to grep; see [Feature design workflow](#feature-design-workflow-new-capabilities). Never log secrets or raw tokens.
- **`src/openhuman/`**: New features go in a **folder/module**, not new root-level `src/openhuman/*.rs` files (see Rust core section).
- **File size**: Prefer ≤ ~500 lines per source file; split modules when growing.
- **Pre-merge checks** (when touching code): Prettier, ESLint, `tsc --noEmit` in `app/`; `cargo fmt` + `cargo check` for changed Rust (`Cargo.toml` at root and/or `app/src-tauri/Cargo.toml` as appropriate).
- **No dynamic imports** in production **`app/src`** code — use **static** `import` / `import type` at the top of the module. Do **not** use `import()` (async dynamic import), `React.lazy(() => import(...))`, or `await import('…')` to load app modules, Tauri APIs, or RPC clients. **Why:** predictable chunk graph, simpler static analysis, fewer surprises in Tauri + Vite, and easier code review. **If a module must not run at load time** (e.g. heavy optional path), use a static import and **guard the call site** with `try/catch` or an explicit runtime check instead of deferring module load via dynamic import. **Exceptions:** Vitest harness patterns (`vi.importActual`, dynamic imports **only** inside `*.test.ts` / `__tests__` / `test/setup.ts` when required by the runner); ambient `typeof import('…')` in `.d.ts`; config files (e.g. `tailwind.config.js` JSDoc).- **Type-only imports**: `import type` where appropriate.
- **Dual socket / tool sync**: If you change realtime protocol, keep **frontend** (`socketService` / MCP transport) and **core** socket behavior aligned (see [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) dual-socket section).

---

## Platform notes

- **macOS deep links**: Often require a built **`.app`** bundle; not only `tauri dev`. See [`docs/telegram-login-desktop.md`](docs/telegram-login-desktop.md) if applicable.
- **`window.__TAURI__`**: Not assumed at module load; guard Tauri usage accordingly.
- **Core sidecar**: Must be staged/built so `core_rpc` can reach the `openhuman` binary (see `scripts/stage-core-sidecar.mjs`).

---

_Last aligned with monorepo layout (`app/` + root `src/`), QuickJS skills in `openhuman_core`, skills catalog on GitHub (`tinyhumansai/openhuman-skills`), and Tauri shell IPC as of repo state._
