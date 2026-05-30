# OpenHuman

**AI assistant for communities — React + Tauri v2 desktop app with a Rust core (JSON-RPC / CLI).**

Narrative architecture: [`gitbooks/developing/architecture.md`](gitbooks/developing/architecture.md). Frontend: [`gitbooks/developing/architecture/frontend.md`](gitbooks/developing/architecture/frontend.md). Tauri shell: [`gitbooks/developing/architecture/tauri-shell.md`](gitbooks/developing/architecture/tauri-shell.md). Agent-harness tool surface: [`gitbooks/developing/architecture/agent-harness.md`](gitbooks/developing/architecture/agent-harness.md).

---

## Repository layout

| Path | Role |
| --- | --- |
| **`app/`** | pnpm workspace `openhuman-app` (v0.53.45): Vite + React (`app/src/`), Tauri desktop host (`app/src-tauri/`), Vitest tests |
| **`src/`** (root) | Rust lib crate `openhuman` + `openhuman-core` CLI binary (`src/main.rs`) — `src/core/` (transport: Axum/HTTP, JSON-RPC, CLI), `src/openhuman/*` domains, event bus |
| **`Cargo.toml`** (root) | Core crate; `cargo build --bin openhuman-core` produces the binary. Also defines `slack-backfill` and `gmail-backfill-3d` helper binaries in `src/bin/`. |
| **`docs/`** | Remaining deep internals (memory pipeline excalidraws, sentry, etc.). Public contributor docs live in `gitbooks/developing/`. |

Commands assume the **repo root**; `pnpm dev` delegates to the `app` workspace. The root `package.json` is `openhuman-repo` (private) and enforces pnpm via the `packageManager` field.

---

## Runtime scope

- **Shipped product**: desktop — Windows, macOS, Linux.
- **Tauri host** (`app/src-tauri`): desktop-only. No Android/iOS branches.
- **Core runs in-process** inside the Tauri host as a tokio task — there is **no sidecar binary anymore** (removed in PR #1061). The lifecycle is owned by `core_process::CoreProcessHandle` in `app/src-tauri/src/core_process.rs`; on Cmd+Q the core dies with the GUI. Frontend RPC still goes over HTTP (`core_rpc_relay` + `core_rpc` client) to `http://127.0.0.1:<port>/rpc`, authenticated with a per-launch bearer the shell hands the embedded server in-memory via `run_server_embedded_with_ready(rpc_token: Some(_))`. The renderer reads the same bearer via the `core_rpc_token` Tauri command. `OPENHUMAN_CORE_TOKEN` is still honoured for CLI / docker / cloud env-as-config (operator-supplied) but is no longer set on the process env by the desktop shell. Set `OPENHUMAN_CORE_REUSE_EXISTING=1` to attach to an externally-started `openhuman-core` process (e.g. a debug harness).

**Where logic lives**
- **Rust core**: business logic, execution, domains, RPC, persistence, CLI. Authoritative.
- **Tauri + React (`app/`)**: UX, screens, navigation, bridging to the core. Presents and orchestrates only.

---

## iOS client (experimental)

The iOS client is an **in-progress, non-shipping** target in this repo. It does not ship a Rust core on-device; instead it connects to the desktop core via one of three transports selected by a `ConnectionProfile`.

**Transport strategies** (see `app/src/services/transport/`):
- `LanHttpTransport` — direct HTTP to the desktop core on the same LAN.
- `TunnelTransport` — socket.io relay through the backend; E2E encrypted with XChaCha20-Poly1305 over X25519 key agreement.
- `CloudHttpTransport` — fallback via the cloud backend API.

**Key paths:**
- PTT plugin: `packages/tauri-plugin-ptt/` (Swift + Rust, iOS-only).
- iOS screens: `app/src/pages/ios/` and `app/src/components/ios/`.
- Devices domain (Rust): `src/openhuman/devices/`.
- Tunnel crypto (TS): `app/src/lib/tunnel/`.
- iOS build entry: `pnpm tauri:ios:dev` — uses stock `@tauri-apps/cli@^2` via `npx`, **not** the vendored CEF CLI.
- Setup guide: `docs/ios/SETUP.md`.

**Backend dependency:** `tinyhumansai/backend#709` (tunnel socket.io contract) must be merged and deployed for end-to-end pairing to work.

---

## Commands (from repo root)

```bash
pnpm dev                  # Vite dev server only (app workspace)
pnpm dev:app              # Full Tauri desktop dev (CEF runtime, loads env via scripts/load-dotenv.sh)
pnpm build                # Production UI build
pnpm typecheck            # tsc --noEmit (app workspace, aliased to `compile`)
pnpm compile              # Same as typecheck
pnpm lint                 # ESLint --cache
pnpm format               # Prettier write + cargo fmt
pnpm format:check         # Prettier check + cargo fmt --check

# Rust — core library + CLI
cargo check --manifest-path Cargo.toml
cargo build --manifest-path Cargo.toml --bin openhuman-core

# Rust — Tauri shell
cargo check --manifest-path app/src-tauri/Cargo.toml
pnpm rust:check           # Tauri shell check
```

Note: `pnpm core:stage` is a no-op (echoes a message). The sidecar was removed in PR #1061; core is linked in-process.

**Tests**: `pnpm test` (Vitest, app workspace) · `pnpm test:coverage` · `pnpm test:rust` (cargo test via `scripts/test-rust-with-mock.sh`).
**Quality**: ESLint + Prettier + Husky in `app`. Pre-push hook runs `pnpm rust:check` — pass `--no-verify` only for unrelated pre-existing breakage.

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

**Agent access mode** — the `[autonomy]` block (`src/openhuman/config/schema/autonomy.rs`) drives the agent's filesystem/shell reach via `SecurityPolicy` (`src/openhuman/security/policy.rs`). Tiers: `level` (`readonly` = read-only / `supervised` = "ask before edit" / `full` = full access) × `workspace_only` × `trusted_roots` (per-folder `read`/`readwrite` grants outside the workspace, overriding `forbidden_paths` for their subtree) × `allow_tool_install` (gates `install_tool`). Edit live via the `config.update_autonomy_settings` RPC or **Settings → Agent access** (`AgentAccessPanel.tsx`); changes swap the process-global policy in `security::live_policy` and apply to new sessions. The default projects home is `~/OpenHuman/projects` (`config::default_projects_dir`, env `OPENHUMAN_PROJECTS_DIR`), auto-created at startup and injected as a ReadWrite trusted root — distinct from the hidden internal `~/.openhuman/workspace`.

**Command permission model (deterministic, fail-closed):** `classify_command` buckets a command into `CommandClass` (`Read` / `Write` / `Network` / `Install` / `Destructive`); an unrecognized command is **`Write`**, never `Read`. `gate_decision(class, tier)` → `Allow` / `Prompt` / `Block`: read-only allows only reads; ask-before-edit prompts every act (file *create* is free, *edit-existing* prompts); full runs read+write but **always-asks** Network/Install/Destructive. Acting tools (`shell`/`node_exec`/`npm_exec`/`file_write`/`edit_file`/`apply_patch`/`git_operations`/`curl`) return `external_effect_with_args() == true` for `Prompt` classes so the harness routes them through the `ApprovalGate` *before* `execute()`; read-only `Block` + structural guards (`check_gated_command`) are enforced in-tool. The LLM may pass a `category` (escalate-only: `max(rust_floor, declared)`). System/credential dirs are an **unconditional** cross-platform block (`is_always_forbidden`, trusted-root-proof). Enforcement is in Rust (`classify_command`/`gate_decision`/`check_gated_command`/`is_path_string_allowed`/`validate_path`), never the system prompt.

> ⚠️ **The approval prompt is ON by default** (opt out with `OPENHUMAN_APPROVAL_GATE=0`/`false`, `jsonrpc.rs`). `ApprovalGate::init_global` installs unless disabled, so `try_global()` is `Some` and the prompt is wired end-to-end; with `OPENHUMAN_APPROVAL_GATE=0` the harness skips the intercept and `Prompt`-class calls **run unprompted**. The gate parks only for **interactive chat turns** (a `tokio` task-local chat context is set in `channels/providers/web.rs`; background triage/cron turns carry no context and are allowed through, not gated). It publishes `DomainEvent::ApprovalRequested`, which `ApprovalSurfaceSubscriber` bridges to the `approval_request` web-channel socket event; the frontend (`ChatApprovalRequestEvent` → `chatRuntime.pendingApprovalByThread` → `ApprovalRequestCard` above the composer) surfaces Approve/Deny, routing to the `openhuman.approval_decide` RPC. A typed `yes`/`no` chat reply is also honoured server-side (web.rs ingress router runs before the "newer request aborts the in-flight turn" path); any other text cancels the parked turn and is taken as a fresh message. Unanswered prompts still park to the 10-min TTL → Deny. Read-only blocking, path hardening, structural guards, and classification **are** live regardless of the flag. Full access ships as documented full-trust (not sandboxed).

---

## Testing

### Unit (Vitest)

- Co-locate as `*.test.ts` / `*.test.tsx` under `app/src/**`.
- Config: `app/test/vitest.config.ts`; setup: `app/src/test/setup.ts`.
- Run from repo root: `pnpm test` or `pnpm test:coverage`. (Inside `app/`, `pnpm test:unit` is also defined.)
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

### Deterministic core reset (E2E)

`app/scripts/e2e-run-spec.sh` creates and cleans a temp `OPENHUMAN_WORKSPACE` by default. `OPENHUMAN_WORKSPACE` redirects core config + storage away from `~/.openhuman`. Each spec gets a fresh in-process core inside the freshly-built Tauri bundle.

### Rust tests with mock

```bash
pnpm test:rust
bash scripts/test-rust-with-mock.sh --test json_rpc_e2e
```

---

## Frontend (`app/src/`)

**Provider chain** (`App.tsx`):
`Sentry.ErrorBoundary` → `Redux Provider` → `PersistGate` (with `PersistRehydrationScreen`) → `BootCheckGate` → `CoreStateProvider` → `SocketProvider` → `ChatRuntimeProvider` → `HashRouter` → `CommandProvider` → `ServiceBlockingGate` → `AppShell` (`AppRoutes` + `BottomTabBar` + walkthrough/mascot/snackbars).

No `UserProvider` / `AIProvider` / `SkillProvider` — auth and core snapshot live in `CoreStateProvider`, fetched via `fetchCoreAppSnapshot()` RPC (auth tokens are NOT in redux-persist; they live in the in-process core).

**State** (`store/`): Redux Toolkit slices — `accounts`, `channelConnections`, `chatRuntime`, `coreMode`, `deepLinkAuth`, `mascot`, `notification`, `providerSurface`, `socket`, `thread`. Persisted slices via redux-persist. Prefer Redux over ad-hoc `localStorage` (exception: ephemeral UI state like upsell dismiss flags).

**Services** (`services/`): singletons — `apiClient`, `socketService`, `coreRpcClient` + `coreCommandClient` (HTTP bridge to in-process core via Tauri IPC), `chatService`, `analytics`, `notificationService`, `webviewAccountService`, `daemonHealthService`, plus domain `api/*` clients.

**MCP** (`lib/mcp/`): JSON-RPC transport, validation, types over Socket.io.

**Routing** (`AppRoutes.tsx`, HashRouter): `/` (Welcome), `/onboarding/*`, `/home`, `/human`, `/intelligence`, `/skills`, `/chat` (unified agent + connected web apps, replaces old `/conversations` + `/accounts`), `/channels`, `/invites`, `/notifications`, `/rewards`, `/webhooks` (redirects to `/settings/webhooks-triggers`), `/settings/*`. Default catch-all is `DefaultRedirect`. There is no `/login`, no `/mnemonic` (recovery phrase moved to Settings), no `/agents`, no `/conversations`.

**AI config**: bundled prompts in `src/openhuman/agent/prompts/` (also bundled via `app/src-tauri/tauri.conf.json` `resources`). Loaders in `app/src/lib/ai/` use `?raw` imports, optional remote fetch, and `ai_get_config` / `ai_refresh_config` in Tauri.

---

## Tauri shell (`app/src-tauri/`)

Thin desktop host. Top-level modules: `core_process`, `core_rpc`, `cdp`, `cef_preflight`, `cef_profile`, `dictation_hotkeys`, `file_logging`, `mascot_native_window`, `native_notifications`, `notification_settings`, `process_kill`, `process_recovery`, `screen_capture`, `window_state`, plus the per-provider scanner modules (`discord_scanner`, `gmessages_scanner`, `imessage_scanner`, `meet_scanner`, `slack_scanner`, `telegram_scanner`, `whatsapp_scanner`), `meet_audio` / `meet_call` / `meet_video`, `fake_camera`, `webview_accounts`, `webview_apis`.

**Core lifecycle**: `core_process::CoreProcessHandle` spawns the JSON-RPC server as an in-process tokio task and authenticates inbound RPC with a per-launch hex bearer. The bearer is generated in `CoreProcessHandle::new()` and handed to the embedded server in-memory through `run_server_embedded_with_ready(rpc_token: Some(_))` — never set on the process env. On stale-listener detection (#1130) the handle revalidates the PID before force-killing so PID reuse can't kill an unrelated process. `restart_core_process` / `start_core_process` Tauri commands let the frontend cycle it for updates.

Registered IPC (see [`gitbooks/developing/architecture/tauri-shell.md`](gitbooks/developing/architecture/tauri-shell.md)) includes `greet`, `write_ai_config_file`, `ai_get_config`, `ai_refresh_config`, `core_rpc_relay`, `core_rpc_token`, `start_core_process`, `restart_core_process`, window commands, and `openhuman_*` daemon helpers. Always use `invoke('core_rpc_relay', ...)` for in-process RPC (avoids CORS preflight that `fetch()` would trigger).

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

- **`src/openhuman/`** — Domain logic. Current domains: `about_app`, `accessibility`, `agent`, `app_state`, `approval`, `autocomplete`, `billing`, `channels`, `composio`, `config`, `context`, `cost`, `credentials`, `cron`, `doctor`, `embeddings`, `encryption`, `health`, `heartbeat`, `integrations`, `learning`, `local_ai`, `meet`, `meet_agent`, `memory`, `migration`, `node_runtime`, `notifications`, `overlay`, `people`, `prompt_injection`, `provider_surfaces`, `providers`, `redirect_links`, `referral`, `routing`, `scheduler_gate`, `screen_intelligence`, `security`, `service`, `skills`, `socket`, `subconscious`, `team`, `text_input`, `threads`, `tokenjuice`, `tool_timeout`, `tools`, `tree_summarizer`, `update`, `voice`, `wallet`, `webhooks`, `webview_accounts`, `webview_apis`, `webview_notifications`. RPC controllers in per-domain `rpc.rs` / `schemas.rs`; use `RpcOutcome<T>` per [`AGENTS.md`](AGENTS.md).
- **Skills runtime removed**: the QuickJS / `rquickjs` runtime that previously executed skill packages is gone. `src/openhuman/skills/` is now a metadata-only domain (`ops_create`, `ops_discover`, `ops_install`, `ops_parse`, `inject`, `schemas`, `types`) — see the module header comment "Legacy skill metadata helpers retained after QuickJS runtime removal."
- **Module layout rule**: new functionality goes in a **dedicated subdirectory** (`openhuman/<domain>/mod.rs` + siblings). **Do not** add new standalone `*.rs` files at `src/openhuman/` root (`dev_paths.rs` and `util.rs` are grandfathered, not a template).
- **Controller schema contract**: shared types in `src/core/types.rs` / `src/core/mod.rs` (`ControllerSchema`, `FieldSchema`, `TypeSchema`).
- **Domain schema files**: per-domain `schemas.rs` (e.g. `src/openhuman/cron/schemas.rs`), exported from domain `mod.rs`.
- **Controller-only exposure**: expose features to CLI and JSON-RPC via the controller registry. **Do not** add domain branches in `src/core/cli.rs` / `src/core/jsonrpc.rs`.
- **Light `mod.rs`**: keep domain `mod.rs` export-focused. Operational code in `ops.rs`, `store.rs`, `types.rs`, etc. See **Canonical module shape** below for the full per-file contract.
- **`src/core/`** — Transport only. Modules: `all`, `all_tests`, `auth`, `autocomplete_cli_adapter`, `cli`, `cli_tests`, `dispatch`, `event_bus/`, `jsonrpc`, `jsonrpc_tests`, `legacy_aliases`, `logging`, `memory_cli`, `observability`, `rpc_log`, `shutdown`, `socketio`, `types`, plus `agent_cli`. No heavy domain logic here. (There is no `src/core_server/` — older docs that reference `core_server` mean `src/core/`.)

### Canonical module shape

Each high-level domain under `src/openhuman/<domain>/` should follow this file contract. Only `mod.rs` and tests are universal; the rest exist **only when applicable** — do not create empty placeholder files (e.g. a stateless domain has no `store.rs`, a domain that exposes no agent tools has no `tools.rs`).

| File | When | Role |
| --- | --- | --- |
| `mod.rs` | always | Export-focused **only**: module docstring + `mod`/`pub mod` decls + `pub use` re-exports, plus the `all_<domain>_controller_schemas` / `all_<domain>_registered_controllers` pair when RPC-facing. **No business logic, no domain-state statics, no domain `impl` blocks.** |
| `types.rs` | domain has its own types | Serde domain types. |
| `store.rs` | domain persists state | Persistence layer. |
| `ops.rs` | domain has logic / handlers | Business logic + entry points returning `RpcOutcome<T>`. **Canonical handler file** (`ops.rs` is the majority convention; `rpc.rs` is legal only where a domain separates a pure-domain API from ops, e.g. `cron` does `pub use ops as rpc`). |
| `schemas.rs` | RPC-facing | Controller schemas + `handle_*` fns delegating to `ops.rs` (see **Controller migration checklist**). |
| `tools.rs` | domain owns agent tools | Domain-owned tool impls live here (+ optional `tools/` submodules), re-exported via `src/openhuman/tools/mod.rs` (see AGENTS.md "Tool ownership rule"). Only genuinely cross-cutting tool families (filesystem, browser/computer, generic system/network) stay in `src/openhuman/tools/impl/`. |
| `bus.rs` | domain has event subscribers | `EventHandler` impls (see **Event bus**). |
| tests | new/changed behavior | Inline `#[cfg(test)] mod tests` (small modules) **or** a sibling `<file>_tests.rs` via `#[cfg(test)] #[path = "<file>_tests.rs"] mod tests;` (large suites). Both are legal. |

Two clarifications:
- **Inline tests do not count against "light `mod.rs`".** A `mod.rs` whose non-test body is pure re-exports is already compliant; moving a large inline suite into a sibling `mod_tests.rs` is tidiness, not a correctness requirement.
- **Narrow thin-facade exception:** pure dispatch forwarders (pick an implementation and forward — no domain state, no I/O of their own) MAY stay in `mod.rs` when being that facade is the module's whole purpose (e.g. `cwd_jail::spawn` / `spawn_with` / `default_backend`). Justify it in the module docstring.

### Controller migration checklist

- `src/openhuman/<domain>/mod.rs`: add `mod schemas;`, re-export `all_controller_schemas as all_<domain>_controller_schemas` and `all_registered_controllers as all_<domain>_registered_controllers`.
- `src/openhuman/<domain>/schemas.rs` defines `schemas`, `all_controller_schemas`, `all_registered_controllers`, and `handle_*` fns delegating to domain `rpc.rs`.
- Wire exports into `src/core/all.rs`. Remove migrated branches from `src/core/dispatch.rs`.

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

This file is loaded into every contributor's Claude Code session, so the instructions below are written generically: `<your-username>` means **your** GitHub username (the owner of your fork), not any specific maintainer. Adapt the literal commands accordingly.

**One-time remote setup.** Contribute via your own fork of `tinyhumansai/openhuman`. Recommended remote layout:

```
origin    git@github.com:<your-username>/openhuman.git  (your fork — push here)
upstream  git@github.com:tinyhumansai/openhuman.git     (fetch-only; never push)
```

If you cloned the upstream directly, fix it once:

```bash
git remote rename origin upstream
git remote add origin git@github.com:<your-username>/openhuman.git
git fetch upstream
```

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the full new-contributor walkthrough.

- **Never write code on `main`.** Before making any code changes, branch off the latest upstream `main` (`git fetch upstream && git checkout -b <branch> upstream/main`). All work happens on that feature branch; `main` stays clean and only advances via merged PRs.
- Issues and PRs on upstream **[tinyhumansai/openhuman](https://github.com/tinyhumansai/openhuman)** — not a fork — unless explicitly told otherwise.
- Issue templates: [`.github/ISSUE_TEMPLATE/feature.md`](.github/ISSUE_TEMPLATE/feature.md), [`.github/ISSUE_TEMPLATE/bug.md`](.github/ISSUE_TEMPLATE/bug.md). PR template: [`.github/PULL_REQUEST_TEMPLATE.md`](.github/PULL_REQUEST_TEMPLATE.md). AI-authored text should follow them verbatim.
- PRs target **`main`** of `tinyhumansai/openhuman`.
- **Push branches to `origin` (your fork), never to `upstream` (`tinyhumansai/openhuman`).** PRs are opened against `tinyhumansai/openhuman:main` with `--head <your-username>:<branch>` so the source is the fork. Direct pushes to upstream pollute its branch list and skip code-review boundaries. Treat the `upstream` remote as fetch-only.
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
- **i18n for all UI text**: every user-visible string in `app/src/**` (headings, labels, button text, placeholders, status chips, toasts, error messages, dialog copy) must go through `useT()` from `app/src/lib/i18n/I18nContext`. Hard-coded literals in JSX or `label=`/`placeholder=`/`aria-label=` props are not allowed. Add the key to [`app/src/lib/i18n/en.ts`](app/src/lib/i18n/en.ts) **and a real translation to every other locale file** in the same PR (see the next bullet — English-only fallback is the runtime safety net for *missing* keys, not an excuse to ship untranslated values). Exceptions: developer-only debug logs, code identifiers, and non-display data (URLs, slugs, technical sentinel values).
- **i18n locale files — update ALL locales with REAL translations**: each locale is a **single flat file** at `app/src/lib/i18n/<locale>.ts` (`en.ts` is the source of truth; the chunked `chunks/<locale>-N.ts` layout was retired). When adding or changing keys in `en.ts`, you **must also** add the same key to every non-English locale file **with an actual, correct translation in that language — NOT an English placeholder**. Do not copy the English string into `ar.ts`/`de.ts`/… and "leave it for translators"; translate it (the short UI strings here are well within reach — translate them, and only fall back to English for genuine brand names / commands / paths / units, which belong in the `INTENTIONAL_ENGLISH` allowlist). CI enforces this two ways: `pnpm i18n:check` fails on a missing or extra key in any locale (parity), and **`pnpm i18n:english:check`** ([`scripts/i18n-find-english.ts`](scripts/i18n-find-english.ts)) fails on any value still rendering English — including *stale* English (translated from an older en string that since changed). The English-detection gate uses script-coverage for non-Latin locales and English-only function words for Latin locales; the `INTENTIONAL_ENGLISH` allowlist is **only** for genuinely-English literals (brand names / commands / paths / units / cognates) — never use it to silence an untranslated string. Locales: `ar`, `bn`, `de`, `es`, `fr`, `hi`, `id`, `it`, `ko`, `pl`, `pt`, `ru`, `zh-CN`.

---

## Platform notes

- **Vendored CEF-aware `tauri-cli`**: runtime is CEF; only the vendored CLI at `app/src-tauri/vendor/tauri-cef/crates/tauri-cli` bundles Chromium into `Contents/Frameworks/`. Stock `@tauri-apps/cli` produces a broken bundle (panic in `cef::library_loader::LibraryLoader::new`). `pnpm dev:app` and all `cargo tauri` scripts call `pnpm tauri:ensure` which runs [`scripts/ensure-tauri-cli.sh`](scripts/ensure-tauri-cli.sh). If overwritten, reinstall with `cargo install --locked --path app/src-tauri/vendor/tauri-cef/crates/tauri-cli`.
- **macOS deep links**: often require a built `.app` bundle, not just `tauri dev`.
- **Windows deep links**: `openhuman://` is registered to `HKCU\Software\Classes\openhuman\shell\open\command` by `tauri-plugin-deep-link::register_all` at first launch (per-user, no UAC). The Tauri shell now reads that key back after `register_all` returns and emits `log::error!` with the actual state (`NotRegistered` / `MissingCommand` / `Stale` / `ReadError`) when the value is missing or doesn't point at the running exe — without it, OAuth callbacks via `openhuman://auth?…` never reach the app (issue #2699). The check lives in [`app/src-tauri/src/deep_link_registration_check.rs`](app/src-tauri/src/deep_link_registration_check.rs); a manual repair script for affected users is in [`gitbooks/overview/troubleshooting-sign-in.md`](gitbooks/overview/troubleshooting-sign-in.md).
- **Tauri environment guard**: use `isTauri()` (from `app/src/services/webviewAccountService.ts`) or wrap `invoke(...)` in `try/catch`; do not check `window.__TAURI__` directly — it is not present at module load and bypasses the established wrapper contract.
- **Core is in-process** (no sidecar): `core_rpc` reaches the embedded server at `http://127.0.0.1:<port>/rpc` with bearer auth. The Tauri shell hands the bearer to the embedded server in-memory (no `OPENHUMAN_CORE_TOKEN` on the process env). `scripts/stage-core-sidecar.mjs` no longer exists; `pnpm core:stage` is a no-op echo. To run the core standalone for debugging, use `./target/debug/openhuman-core serve` (token at `{workspace}/core.token`, default `~/.openhuman-staging/core.token` under `OPENHUMAN_APP_ENV=staging`); docker / cloud deployments still supply the bearer via `OPENHUMAN_CORE_TOKEN` in the environment (operator-supplied).
