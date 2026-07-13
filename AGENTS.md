# OpenHuman

**AI assistant for communities — React + Tauri v2 desktop app with a Rust core (JSON-RPC / CLI) embedded in-process.**

Architecture docs: [`gitbooks/developing/architecture.md`](gitbooks/developing/architecture.md) | [Frontend](gitbooks/developing/architecture/frontend.md) | [Tauri shell](gitbooks/developing/architecture/tauri-shell.md) | [Agent harness](gitbooks/developing/architecture/agent-harness.md)

---

## Repository layout

| Path                    | Role                                                                                                                          |
| ----------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| **`app/`**              | pnpm workspace `openhuman-app`: Vite + React (`app/src/`), Tauri desktop host (`app/src-tauri/`), Vitest tests                |
| **`src/`** (root)       | Rust lib crate `openhuman` + `openhuman-core` CLI binary (`src/main.rs`) — `src/core/` (transport), `src/openhuman/*` domains |
| **`Cargo.toml`** (root) | Core crate; `cargo build --bin openhuman-core`. Also `slack-backfill` and `gmail-backfill-3d` in `src/bin/`.                  |
| **`docs/`**             | Deep internals. Public contributor docs in `gitbooks/developing/`.                                                            |

Commands assume **repo root**. Root `package.json` is `openhuman-repo` (private, pnpm-enforced).

---

## Runtime scope

- **Shipped product**: desktop — Windows, macOS, Linux. No Android/iOS in the Tauri host.
- **Core runs in-process** as a tokio task (sidecar removed PR #1061). Lifecycle: `core_process::CoreProcessHandle` in `app/src-tauri/src/core_process.rs`. Frontend RPC → `http://127.0.0.1:<port>/rpc` with per-launch hex bearer handed in-memory via `run_server_embedded_with_ready(rpc_token: Some(_))`. Renderer reads bearer via `core_rpc_token` Tauri command. `OPENHUMAN_CORE_TOKEN` still honoured for CLI/docker/cloud. Set `OPENHUMAN_CORE_REUSE_EXISTING=1` for external core debugging.

**Where logic lives:**

- **Rust core** (`src/`): business logic, execution, domains, RPC, persistence, CLI. Authoritative.
- **Tauri + React** (`app/`): UX, screens, navigation, bridging. Presents and orchestrates only.

---

## iOS client (experimental, non-shipping)

Connects to desktop core via `ConnectionProfile` transport strategies in `app/src/services/transport/`: `LanHttpTransport`, `TunnelTransport` (E2E encrypted XChaCha20-Poly1305), `CloudHttpTransport`. Key paths: PTT plugin `packages/tauri-plugin-ptt/`, iOS screens `app/src/pages/ios/`, devices domain `src/openhuman/devices/`, tunnel crypto `app/src/lib/tunnel/`. Build: `pnpm tauri:ios:dev` (stock `@tauri-apps/cli`, not vendored CEF). Backend dep: `tinyhumansai/backend#709`.

---

## Commands (from repo root)

```bash
pnpm dev                  # Vite dev server only
pnpm dev:app              # Full Tauri desktop dev (CEF, loads env via scripts/load-dotenv.sh)
pnpm build                # Production UI build
pnpm typecheck            # tsc --noEmit (alias: compile)
pnpm lint                 # ESLint --cache
pnpm format               # Prettier write + cargo fmt
pnpm format:check         # Prettier check + cargo fmt --check

# Rust
cargo check --manifest-path Cargo.toml
cargo build --manifest-path Cargo.toml --bin openhuman-core
cargo check --manifest-path app/src-tauri/Cargo.toml   # or: pnpm rust:check

# macOS Apple Silicon workaround (whisper-rs / llama.cpp)
GGML_NATIVE=OFF cargo check --manifest-path Cargo.toml
```

`pnpm core:stage` is a no-op (sidecar removed).

**Build speed**: both `Cargo.toml` files set `[profile.dev.package."*"] debug = false` — dependencies compile without DWARF in `dev`/`test` (faster builds + smaller `target/`); our own crates keep full debuginfo so panics/backtraces still resolve to file:line. `release`/`ci` profiles are unchanged. Keep this stanza in sync across the root and `app/src-tauri/Cargo.toml` if you touch profiles.

**Two-lane CI model**: **CI Lite** (`ci-lite.yml`, quick — pushes to `main` + PRs targeting `main` or `release`): quality checks per changed area plus unit tests **only for the changed files** — `vitest related` for `app/src` changes and domain-scoped `cargo llvm-cov` (libtest filter derived from `src/<a>/<b>/…`) for Rust — still gated at ≥ 80% diff coverage. Config-level changes (lockfile, Cargo.toml/lock, vitest config, `src/lib.rs`, …) fall back to the full suite (`scripts/ci/vitest-changed-coverage.sh`, `scripts/ci/rust-coverage-changed.sh`). **CI Full** (`ci-full.yml`, slow — PRs targeting the long-lived `release` branch + every push to it): complete unit suites, Rust mock-backend E2E, Playwright, and the full desktop E2E matrix on 3 OSes, aggregated by the `CI Full Gate` check (except the Playwright spec run — non-blocking signal while flaky, #3615). `release` advances when a maintainer dispatches `promote-main-to-release.yml` (pushes a merge commit from `main` into `release` — no standing PR) and when fix PRs opened directly against `release` merge (those run both lanes, with `CI Full Gate` blocking the merge; the post-merge push re-runs CI Full). Releases are cut from `release` (`release-staging.yml` / `release-production.yml` run there and tag on `release`; production takes a `release_type` increment — patch/minor/major); **every** cut back-merges `release` into `main` via `scripts/release/merge-release-into-main.sh`, and version-bump commits carry `[skip ci]`. Long build/test commands must run through `scripts/ci-cancel-aware.sh`, whose Actions-API watchdog stops cancelled builds inside container jobs (docker exec swallows runner signals).

**CI build topology**: full-suite E2E is **build-once-then-fanout** on all three OSes — `build-{linux,macos,windows}-full` compile/bundle the app once and upload it as a per-run workflow artifact, and the shard jobs (`e2e-*-full`) `needs:` that job and download it instead of each shard rebuilding on a cold cache (`.github/workflows/e2e-reusable.yml`). Linux desktop packaging (`build-desktop.yml`) does a **single** `cargo tauri build`: libcef.so is resolved from the restored CEF cache (or a targeted `cargo build -p cef-dll-sys` prewarm on a cold cache) rather than a throwaway `--no-bundle` full build. The root core crate and the Tauri shell are still **separate Cargo worlds** (two `Cargo.lock`, two `target/`); converging them into one workspace is tracked as follow-up in #3877.

**Tests**: `pnpm test` (Vitest) · `pnpm test:coverage` · `pnpm test:rust` (`scripts/test-rust-with-mock.sh`).
**Quality**: ESLint + Prettier + Husky. Pre-push hook runs `pnpm rust:check`.

### Agent debug runners (`scripts/debug/`)

Summary-sized stdout; full output teed to `target/debug-logs/`. Add `--verbose` to stream raw.

```bash
pnpm debug unit                                    # full Vitest suite
pnpm debug unit src/components/Foo.test.tsx        # one file
pnpm debug unit -t "renders empty state"           # filter by name
pnpm debug e2e test/e2e/specs/smoke.spec.ts        # WDIO E2E
pnpm debug rust                                    # cargo tests
pnpm debug rust json_rpc_e2e                       # targeted
pnpm debug logs                                    # list recent
pnpm debug logs last                               # print most recent
```

### Coverage requirement (merge gate)

PRs need **≥ 80% coverage on changed lines** via `diff-cover` over Vitest + `cargo-llvm-cov` lcov. Enforced by the coverage jobs (`frontend-coverage`/`rust-core-coverage`/`rust-tauri-coverage`/`coverage-gate`) in `.github/workflows/ci-lite.yml`.

---

## Configuration

- **[`.env.example`](.env.example)** — Rust core, Tauri shell, backend URL, logging. Load: `source scripts/load-dotenv.sh`.
- **[`app/.env.example`](app/.env.example)** — `VITE_*` vars. Copy to `app/.env.local`.
- **Frontend config** centralized in [`app/src/utils/config.ts`](app/src/utils/config.ts) — never read `import.meta.env` directly elsewhere.
- **Rust config**: TOML `Config` struct (`src/openhuman/config/schema/types.rs`) with env overrides (`load.rs`).

### Agent access & security

The `[autonomy]` block (`src/openhuman/config/schema/autonomy.rs`) drives `SecurityPolicy` (`src/openhuman/security/policy.rs`). Tiers: `readonly` / `supervised` / `full` × `workspace_only` × `trusted_roots` × `allow_tool_install`. Edit via `config.update_autonomy_settings` RPC or Settings → Agent access.

**Two path roots** (`src/openhuman/config/schema/types.rs`):

- **`action_dir`** — agent's read/write root. Acting tools resolve relative paths here. Default: `~/OpenHuman/projects` (`OPENHUMAN_ACTION_DIR`).
- **`workspace_dir`** — internal state (`~/.openhuman/users/<id>/workspace`). Agent tools **cannot** write here — enforced by `is_workspace_internal_path` fail-closed regardless of tier/trusted_roots.

**Command permission model**: `classify_command` → `CommandClass` (`Read`/`Write`/`Network`/`Install`/`Destructive`); unrecognized = `Write`. `gate_decision(class, tier)` → `Allow`/`Prompt`/`Block`. System/credential dirs unconditionally blocked (`is_always_forbidden`).

**Approval gate** ON by default (opt out: `OPENHUMAN_APPROVAL_GATE=0`). Parks interactive chat turns only; background/cron allowed through. Frontend surfaces via `ApprovalRequestCard`. 10-min TTL → Deny.

**Sandbox backends** (opt-in per agent via `sandbox_mode = "sandboxed"`): Docker (remote/cron), Local OS jail (Landlock/Seatbelt/AppContainer, desktop), Noop fallback. In-Rust path hardening applies regardless.

---

## Testing

### Unit (Vitest)

- Co-locate as `*.test.ts(x)` under `app/src/**`. Config: `app/test/vitest.config.ts`.
- Run: `pnpm test` or `pnpm test:coverage`. Prefer behavior over implementation. No real network, no time flakes.

### Shared mock backend

- Core: `scripts/mock-api-core.mjs` · Server: `scripts/mock-api-server.mjs` · E2E: `app/test/e2e/mock-server.ts`.
- Admin: `GET /__admin/health`, `POST /__admin/reset`, `POST /__admin/behavior`, `GET /__admin/requests`.
- Manual: `pnpm mock:api`.

### E2E (WDIO — dual platform)

Full guide: [`gitbooks/developing/e2e-testing.md`](gitbooks/developing/e2e-testing.md).

- **Linux (CI)**: `tauri-driver` (WebDriver :4444). **macOS (local)**: Appium Mac2 (XCUITest :4723).
- Specs: `app/test/e2e/specs/*.spec.ts`. Use `element-helpers.ts` helpers, never raw `XCUIElementType*`.
- `e2e-run-spec.sh` creates/cleans temp `OPENHUMAN_WORKSPACE` by default.

### Rust tests

```bash
pnpm test:rust
bash scripts/test-rust-with-mock.sh --test json_rpc_e2e
```

---

## Frontend (`app/src/`)

**Provider chain** (`App.tsx`): `Sentry.ErrorBoundary` → `Redux Provider` → `PersistGate` → `BootCheckGate` → `CoreStateProvider` → `SocketProvider` → `ChatRuntimeProvider` → `HashRouter` → `CommandProvider` → `ServiceBlockingGate` → `AppShell`.

No `UserProvider`/`AIProvider`/`SkillProvider` — auth lives in `CoreStateProvider` via `fetchCoreAppSnapshot()` RPC.

**State** (`store/`): Redux Toolkit slices — `accounts`, `agentProfile`, `announcement`, `backendMeet`, `channelConnections`, `chatRuntime`, `companion`, `connectivity`, `coreMode`, `deepLinkAuth`, `layout`, `locale`, `mascot`, `notification`, `persona`, `providerSurface`, `ptt`, `socket`, `theme`, `thread`, `userErrors` (authoritative list: `store/index.ts`; persistence via `userScopedStorage`). Prefer Redux over ad-hoc `localStorage`.

**Services** (`services/`): `apiClient`, `socketService`, `coreRpcClient`, `coreCommandClient`, `chatService`, `analytics`, `notificationService`, `webviewAccountService`, `daemonHealthService`, plus domain `api/*` clients. Always use `coreRpcClient` (which invokes the `relay_http_rpc` Tauri command) for core RPC.

**Routing** (`AppRoutes.tsx`, HashRouter): `/` (Welcome), `/auth`, `/onboarding/*`, `/chat/:threadId?`, `/human`, `/brain` (+ `/brain/tinyplace-orchestration`), `/orchestration`, `/connections`, `/flows` (+ `/flows/:id`, `/flows/draft`), `/agent-world/*`, `/invites`, `/notifications`, `/rewards`, `/settings/*`, `/feedback`. Back-compat redirects: `/home`→`/chat`, `/skills`→`/connections`, `/channels`→`/connections?tab=messaging`, `/intelligence` & `/activity`→`/settings/notifications`, `/routines` & `/workflows`→`/settings/automations`, `/webhooks`→`/settings/integrations#webhooks`. No `/login`, `/mnemonic`, `/agents`, `/conversations`.

**AI config**: bundled prompts in `src/openhuman/agent/prompts/` ship via `tauri.conf.json` resources and are read core-side (`app/src/lib/ai/` holds agent-context helpers, not prompt loaders).

---

## Tauri shell (`app/src-tauri/`)

Thin desktop host. Key modules: `core_process`, `core_rpc`, `cdp`, `cef_preflight`, `cef_profile`, `dictation_hotkeys`, `file_logging`, `mascot_native_window`, `screen_capture`, `window_state`, per-provider scanners (`discord_scanner`, `slack_scanner`, `telegram_scanner`, `whatsapp_scanner`, `wechat_scanner`, `gmessages_scanner`, `imessage_scanner`, `meet_scanner`), `meet_audio`/`meet_call`/`meet_video`, `fake_camera`, `webview_accounts`, `webview_apis`.

IPC commands (authoritative list: `generate_handler!` in `app/src-tauri/src/lib.rs`): `core_rpc::relay_http_rpc`, `core_rpc_url`, `core_rpc_token`, `start_core_process`/`restart_core_process`, update commands (`check_app_update`, `apply_core_update`, …), window commands (`activate_main_window`, `mascot_window_*`, `notch_window_*`), `webview_accounts::*`, `workspace_paths::*`, `artifact_commands::*`, hotkeys (dictation/PTT/companion), `meet_call::*`, `native_notifications::*`, `mcp_commands::*`, `loopback_oauth::*`.

### CEF child webviews — no new JS injection

Embedded provider webviews **must not** grow new JS injection. No new `.js` under `webview_accounts/`, no new `build_init_script`/`RUNTIME_JS` blocks, no CDP `Page.addScriptToEvaluateOnNewDocument`. New behavior lives in CEF handlers, CDP from scanner modules, or Rust-side IPC hooks. Legacy injection (gmail, linkedin, google-meet) is grandfathered but should shrink. Audit new Tauri plugins for `js_init_script` calls.

---

## Rust core (`src/`)

### Domain layout (`src/openhuman/`)

~130 domain directories — authoritative list: `ls -d src/openhuman/*/`. Major families: agent (`agent`, `agent_experience`, `agent_meetings`, `agent_memory`, `agent_orchestration`, `agent_registry`, `agent_tool_policy`, `agentbox`, `orchestration`), memory (`memory`, `memory_archivist`, `memory_conversations`, `memory_diff`, `memory_goals`, `memory_queue`, `memory_search`, `memory_sources`, `memory_store`, `memory_sync`, `memory_tools`, `memory_tree`, `tinycortex`), skills/flows (`skills`, `skill_registry`, `skill_runtime`, `flows`, `tinyflows`, `tinyagents`, `rhai_workflows`), inference/AI (`inference`, `model_council`, `council_registry`, `embeddings`, `routing`), MCP (`mcp_audit`, `mcp_client`, `mcp_registry`, `mcp_server`), runtimes (`runtime_node`, `runtime_python`, `runtime_python_server`, `javascript`, `sandbox`, `cwd_jail`), channels/webviews (`channels`, `webview_accounts`, `webview_apis`, `webview_notifications`, `whatsapp_data`), meet (`meet`, `meet_agent`), web3 (`wallet`, `web3`, `x402`, `tokenjuice`), plus platform domains (`about_app`, `approval`, `config`, `cron`, `credentials`, `keyring`, `security`, `threads`, `tools`, `update`, `voice`, …).

**Skills runtime**: the QuickJS per-skill VM engine is gone. `src/openhuman/skills/` holds skill metadata/tool descriptors; execution of installed `SKILL.md` workflows lives in `src/openhuman/skill_runtime/` (starts/cancels runs, hosts the `skill_executor` agent, reuses `runtime_node`/`runtime_python`).

**Rules:**

- New functionality → dedicated subdirectory (`openhuman/<domain>/mod.rs` + siblings). No new root-level `*.rs` files.
- **Tool ownership**: domain tools live in that domain's `tools.rs`, re-exported via `src/openhuman/tools/mod.rs`. Only cross-cutting families stay in `tools/impl/`.
- **Memory source identity**: per-item IDs are dedupe keys only; set `metadata.path_scope` to stable collection scope.
- **Controller-only exposure**: use the registry, not branches in `cli.rs`/`jsonrpc.rs`.

### Canonical module shape

| File         | When                         | Role                                                                                          |
| ------------ | ---------------------------- | --------------------------------------------------------------------------------------------- |
| `mod.rs`     | always                       | Export-focused only: `mod`/`pub mod` + `pub use` + controller schema pair. No business logic. |
| `types.rs`   | domain has types             | Serde domain types.                                                                           |
| `store.rs`   | domain persists              | Persistence layer.                                                                            |
| `ops.rs`     | domain has logic             | Business logic + handlers returning `RpcOutcome<T>`.                                          |
| `schemas.rs` | RPC-facing                   | Controller schemas + `handle_*` fns delegating to `ops.rs`.                                   |
| `tools.rs`   | domain owns agent tools      | Tool implementations.                                                                         |
| `bus.rs`     | domain has event subscribers | `EventHandler` impls.                                                                         |
| tests        | new/changed behavior         | Inline `#[cfg(test)] mod tests` or sibling `*_tests.rs`.                                      |

### Controller migration checklist

1. `mod.rs`: add `mod schemas;`, re-export `all_controller_schemas`/`all_registered_controllers`.
2. `schemas.rs`: define schemas, handlers delegating to `ops.rs`.
3. Wire into `src/core/all.rs`. Remove from `src/core/dispatch.rs`.

### `src/core/` — transport only

Modules: `all`, `auth`, `cli`, `dispatch`, `event_bus/`, `jsonrpc`, `logging`, `observability`, `types`, etc. No business logic here.

### Runtime composition — `ServiceSet` + `DomainSet` on `CoreBuilder`

Two independent runtime axes on `CoreBuilder` (`src/core/runtime/builder.rs`):

- **`ServiceSet`** selects which *background services / transports* run (`rpc_http`, `socketio`, `cron`, `channels`, `heartbeat`, …). Presets: `desktop()` / `headless_api()` / `none()`.
- **`DomainSet`** selects which *domain families* exist at runtime, one flag per `DomainGroup` (`src/core/all.rs`). Presets: `full()` (default — byte-identical to before #4796), `harness()` (agent + memory + threads + config + security only), `none()`. Every controller is tagged with its `DomainGroup` at the single registration site in `src/core/all.rs`; the live surface (controllers/`/schema`/dispatch, agent tools, stores, subscribers) is filtered by the ambient `CoreContext::domains()`. A gated domain's controllers become unknown-method, its agent tools absent, its stores/subscribers uninitialized. `examples/embed_headless.rs` uses `DomainSet::harness()`. Per-gate Cargo `[features]` (children #4797–#4804) narrow the compile-time surface further; `DomainSet` is the runtime axis they compose with.

### Compile-time domain gates (Cargo `[features]`)

Per-domain Cargo features drop whole domains **at compile time** (smaller binary, fewer deps), composing with the runtime `DomainSet` axis above. Each gate is **default-ON**, so the desktop build is byte-identical; slim builds opt out explicitly.

**Slim-profile convention** (no `full` meta-feature): build slim variants with `cargo build --no-default-features --features "<explicit list of gates you want>"`. This mirrors the existing standalone-feature style (`sandbox-landlock`, `browser-native`, …). Example — everything except voice:

```bash
# check / build without the voice + audio_toolkit domains
GGML_NATIVE=OFF cargo check --manifest-path Cargo.toml \
  --no-default-features --features tokenjuice-treesitter
```

| Feature | Default | Gates | Drops deps |
| ------- | ------- | ----- | ---------- |
| `voice` | ON | `openhuman::voice` + `openhuman::audio_toolkit` domains — STT/TTS providers, dictation server, always-on listening, podcast audio + email | `hound`, `lettre` |

**Facade pattern (pathfinder for the other gates).** `pub mod voice;` is **always compiled** as a facade: the real submodules are `#[cfg(feature = "voice")]`, and a `#[cfg(not(feature = "voice"))] mod stub;` (`src/openhuman/voice/stub.rs`) re-exposes the same public surface that always-on / other-gated callers use (`server`, `dictation_listener`, `streaming`, `reply_speech`, `cloud_transcribe`, `cli`, `create_stt_provider`, `effective_stt_provider`, `publish_ptt_transcript_committed`) with no-op / `None` / disabled-error bodies. Callers therefore do **not** need per-call `#[cfg]`. When voice is off: the voice/audio controllers are unregistered (unknown-method over `/rpc`, absent from `/schema`), the `audio_generate_podcast` agent tools are absent, and `openhuman voice` returns a "voice disabled" error. Stub signatures must match the real ones exactly — the disabled build (`--no-default-features --features tokenjuice-treesitter`) is the **only** thing that catches drift, so run it before pushing any change to the voice surface.

**Scope note:** the `voice` gate does **not** drop `whisper-rs` / `llama` / `cpal`. Those live in the inference domain (`src/openhuman/inference/local/service/whisper_engine.rs`; `cpal` is shared with accessibility) and await a separate future `inference` gate. The issue-level DoD line claiming whisper is dropped is superseded by this scope correction.

### Event bus (`src/core/event_bus/`)

Typed pub/sub + native request/response. Both singletons — use module-level functions.

- **Broadcast** (`publish_global`/`subscribe_global`): fire-and-forget, many subscribers.
- **Native request/response** (`register_native_global`/`request_native_global`): one-to-one typed dispatch, zero serialization, internal-only.

Core types: `DomainEvent` (events.rs), `EventBus` (bus.rs), `NativeRegistry` (native_request.rs), `EventHandler`/`SubscriptionHandle` (subscriber.rs).

Domains: `agent`, `memory`, `channel`, `cron`, `skill`, `tool`, `webhook`, `system`.

Each domain owns `bus.rs` with handlers. Convention: `<Purpose>Subscriber`, `name()` → `"<domain>::<purpose>"`.

**Adding events:** add to `DomainEvent`, extend `domain()` match, create `<domain>/bus.rs`, register at startup, publish via `publish_global`.

**Adding native handlers:** define req/resp types (`Send + 'static`, not `Serialize`), register at startup keyed by `"<domain>.<verb>"`, dispatch via `request_native_global`.

---

## Design & patterns

**Visual**: ocean primary `#4A83DD`, sage/amber/coral semantics, Inter + Cabinet Grotesk + JetBrains Mono. Tokens in [`app/tailwind.config.js`](app/tailwind.config.js).

**Key rules:**

- File size: prefer ≤ ~500 lines.
- **No dynamic imports** in production `app/src` — static `import`/`import type` only. Guard heavy paths with try/catch. Exceptions: test files, `.d.ts`, config files.
- **i18n**: all UI text through `useT()` from `app/src/lib/i18n/I18nContext`. Add key to `en.ts` **and real translations to all locale files** (`ar`, `bn`, `de`, `es`, `fr`, `hi`, `id`, `it`, `ko`, `pl`, `pt`, `ru`, `zh-CN`). CI enforces parity (`pnpm i18n:check`) and detects English placeholders (`pnpm i18n:english:check`).
- **Dual socket sync**: keep `socketService`/MCP transport aligned with core socket behavior.
- **Tauri guard**: use `isTauri()` or wrap `invoke(...)` in try/catch — never check `window.__TAURI__` directly.
- **Generated docs**: some architecture docs contain generated blocks marked `<!-- BEGIN/END GENERATED: … -->` sourced from code (today: the frontend provider chain in [`gitbooks/developing/architecture/frontend.md`](gitbooks/developing/architecture/frontend.md), from the `@generated-source:provider-chain` marker in `app/src/App.tsx`). Don't hand-edit between the markers — update the code source, then run `pnpm docs:generate`. CI (`pnpm docs:check`, the **Docs Drift** lane) fails on stale generated docs. Generator + tests: `scripts/generate-architecture-docs.mjs`.

---

## Debug logging (must follow)

- Default to **verbose diagnostics** on new/changed flows.
- Log entry/exit, branches, external calls, retries/timeouts, state transitions, errors.
- Stable grep-friendly prefixes (`[domain]`, `[rpc]`), correlation fields (request IDs, method names).
- Rust: `log`/`tracing` at `debug`/`trace`. App: namespaced `debug`.
- **Never** log secrets or full PII.
- Changes lacking logging are incomplete.

---

## Feature design workflow

Specify → prove in Rust → prove over RPC → surface in UI → test.

1. **Specify** — ground in existing domains, controller patterns, JSON-RPC naming (`openhuman.<namespace>_<function>`).
2. **Implement in Rust** — domain logic + unit tests.
3. **JSON-RPC E2E** — extend `tests/json_rpc_e2e.rs` / `scripts/test-rust-with-mock.sh`.
4. **UI** — React + `coreRpcClient` (`relay_http_rpc`). Keep rules in core.
5. **App unit tests** — Vitest.
6. **App E2E** — desktop specs.

Update `src/openhuman/about_app/` when adding/removing/renaming user-facing features. Define E2E scenarios up front covering happy paths, failures, auth gates.

---

## Git workflow

Contribute via your fork. Recommended remotes:

```text
origin    git@github.com:<your-username>/openhuman.git  (push here)
upstream  git@github.com:tinyhumansai/openhuman.git     (fetch-only)
```

- **Never write code on `main`.** Branch off `upstream/main` for all work.
- Issues and PRs on upstream `tinyhumansai/openhuman`.
- Push to `origin` (fork), never `upstream`. PRs with `--head <your-username>:<branch>`.
- Use issue/PR templates verbatim.
- On push blockers: fix your own hook failures; bypass with `--no-verify` only for unrelated pre-existing breakage (call out in PR body).

---

## Platform notes

- **Vendored CEF-aware `tauri-cli`**: only the vendored CLI at `app/src-tauri/vendor/tauri-cef/crates/tauri-cli` bundles Chromium correctly. Stock `@tauri-apps/cli` produces broken bundles. Reinstall: `cargo install --locked --path app/src-tauri/vendor/tauri-cef/crates/tauri-cli`.
- **macOS deep links**: require built `.app` bundle, not just `tauri dev`.
- **Windows deep links**: `openhuman://` registered via `tauri-plugin-deep-link::register_all`. Check in `app/src-tauri/src/deep_link_registration_check.rs`.
- **Core standalone debugging**: `./target/debug/openhuman-core serve` (token at `{workspace}/core.token`). Public endpoints: `GET /health`, `GET /schema`, `GET /events`.

---

## Coding philosophy

- **Unix-style modules**: small, single-responsibility, composed through clear boundaries.
- **Tests before the next layer**: untested code is incomplete.
- **Docs with code**: update AGENTS.md or architecture docs when rules or behavior change.
