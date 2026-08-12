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

Connects to desktop core via `ConnectionProfile` transport strategies in `app/src/services/transport/`: `LanHttpTransport`, `TunnelTransport` (E2E encrypted XChaCha20-Poly1305), `CloudHttpTransport`. Key paths: PTT plugin `packages/tauri-plugin-ptt/`, iOS screens `app/src/pages/ios/`, devices domain `src/openhuman/security/devices/`, tunnel crypto `app/src/lib/tunnel/`. Build: `pnpm tauri:ios:dev` (stock `@tauri-apps/cli`, not vendored CEF). Backend dep: `tinyhumansai/backend#709`.

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

# macOS Apple Silicon workaround (llama.cpp)
GGML_NATIVE=OFF cargo check --manifest-path Cargo.toml
```

`pnpm core:stage` is a no-op (sidecar removed).

**Build speed**: both `Cargo.toml` files set `[profile.dev.package."*"] debug = false` — dependencies compile without DWARF in `dev`/`test` (faster builds + smaller `target/`); our own crates keep full debuginfo so panics/backtraces still resolve to file:line. `release`/`ci` profiles are unchanged. Keep this stanza in sync across the root and `app/src-tauri/Cargo.toml` if you touch profiles.

**Two-lane CI model**: **CI Lite** (`ci-lite.yml`, quick — pushes to `main` + PRs targeting `main` or `release`): quality checks per changed area plus unit tests **only for the changed files** — `vitest related` for `app/src` changes and domain-scoped `cargo llvm-cov` (libtest filter derived from `src/<a>/<b>/…`) for Rust — still gated at ≥ 80% diff coverage. Config-level changes (lockfile, Cargo.toml/lock, vitest config, `src/lib.rs`, …) fall back to the full suite (`scripts/ci/vitest-changed-coverage.sh`, `scripts/ci/rust-coverage-changed.sh`). **CI Full** (`ci-full.yml`, slow — PRs targeting the long-lived `release` branch + every push to it): complete unit suites, Rust mock-backend E2E, Playwright, and the full desktop E2E matrix on 3 OSes, aggregated by the `CI Full Gate` check (except the Playwright spec run — non-blocking signal while flaky, #3615). `release` advances when a maintainer dispatches `promote-main-to-release.yml` (pushes a merge commit from `main` into `release` — no standing PR) and when fix PRs opened directly against `release` merge (those run both lanes, with `CI Full Gate` blocking the merge; the post-merge push re-runs CI Full). Production releases are always cut from `release`; staging builds may be cut from `main` or `release` by selecting that workflow-dispatch ref. Release-source cuts back-merge `release` into `main` via `scripts/release/merge-release-into-main.sh`, and version-bump commits carry `[skip ci]`. Long build/test commands must run through `scripts/ci-cancel-aware.sh`, whose Actions-API watchdog stops cancelled builds inside container jobs (docker exec swallows runner signals).

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

**Analytics**: use `Button analyticsId="stable-content-free-id"` for shared button interactions, `AnalyticsPageTracker` once inside the router, and `trackAnalyticsEvent` from `components/analytics` for successful domain outcomes (messages, automation runs, connections, etc.). Native controls and links may use `data-analytics-id` directly. Use privacy-safe dimensions only; never send user-authored text, entity IDs, filenames, credentials, or error messages. `services/analytics.ts` is the consent/provider implementation, not the feature-code API.

**Routing** (`AppRoutes.tsx`, HashRouter): `/` (Welcome), `/auth`, `/onboarding/*`, `/chat/:threadId?`, `/human`, `/brain` (+ `/brain/tinyplace-orchestration`), `/orchestration`, `/connections`, `/flows` (+ `/flows/:id`, `/flows/draft`), `/agent-world/*`, `/invites`, `/notifications`, `/rewards`, `/settings/*`, `/feedback`. Back-compat redirects: `/home`→`/chat`, `/skills`→`/connections`, `/channels`→`/connections?tab=messaging`, `/intelligence` & `/activity`→`/settings/notifications`, `/routines` & `/workflows`→`/settings/automations`, `/webhooks`→`/settings/integrations#webhooks`. No `/login`, `/mnemonic`, `/agents`, `/conversations`.

**AI config**: bundled prompts in `src/openhuman/agent/prompts/` ship via `tauri.conf.json` resources and are read core-side (`app/src/lib/ai/` holds agent-context helpers, not prompt loaders).

---

## Tauri shell (`app/src-tauri/`)

Thin desktop host. Key modules: `core_process`, `core_rpc`, `cdp`, `dictation_hotkeys`, `file_logging`, `mascot_native_window`, `window_state`, `imessage_scanner`, `webview_apis`.

The CDP-driven provider scanners (`discord_scanner`, `slack_scanner`, `telegram_scanner`, `whatsapp_scanner`, `wechat_scanner`, `gmessages_scanner`), the `webview_accounts` surface they ran inside, and the in-app Meet call window (`meet_call`, `meet_audio`, `meet_video`, `meet_scanner`, `fake_camera`) were removed in #5478 — CDP only exists under a Chromium engine, and the app moved to Wry in #5456. `imessage_scanner` is unaffected: it reads `chat.db` natively and never used CDP. Meet itself is unaffected too — it runs through `src/openhuman/meet/` and the `backend_bot` (Recall.ai) path, which never touched CDP.

IPC commands (authoritative list: `generate_handler!` in `app/src-tauri/src/lib.rs`): `core_rpc::relay_http_rpc`, `core_rpc_url`, `core_rpc_token`, `start_core_process`/`restart_core_process`, update commands (`check_app_update`, `apply_core_update`, …), window commands (`activate_main_window`, `mascot_window_*`, `notch_window_*`), `workspace_paths::*`, `artifact_commands::*`, hotkeys (dictation/PTT/companion), `native_notifications::*`, `mcp_commands::*`, `loopback_oauth::*`.

### Child webviews — no new JS injection

Child webviews **must not** grow new JS injection. No new `build_init_script` / `RUNTIME_JS` blocks, and no new injected `.js` assets. **New behavior lives in Rust-side IPC hooks.**

That is now the only destination. The rule previously offered three — "CEF handlers, CDP from scanner modules, or Rust-side IPC hooks" — and #5478 removed the first two: there are no CEF handlers (the runtime is Wry as of #5456) and no scanner modules or CDP layer. The surfaces the rule was written to protect (the embedded provider webviews) are gone with them, so today it governs the webviews the shell still owns.

**This is a narrowing, not a licence.** Losing two destinations does not make injection into the remaining webviews acceptable; it means the one sanctioned route is Rust-side IPC. If a future feature genuinely needs page-side script — the plausible candidate is re-serving WhatsApp / WeChat / Google Messages via Wry's `eval`, noted as out of scope in #5478 — that is a **deliberate decision to take first**, not something to read into this paragraph.

Audit new Tauri plugins for `js_init_script` calls.

---

## Rust core (`src/`)

### Extracted host-agnostic crates — `vendor/tinydocs`, `vendor/tinywallet`

Two vendored crates own logic that used to live in this repo. Both are git
submodules consumed by `path` (not published to crates.io, so no
`[patch.crates-io]` entry — same shape as `tinyhumans-sdk`). After cloning:
`git submodule update --init vendor/tinydocs vendor/tinywallet`.

The split follows one rule, and it is worth stating because it decides where
the *next* extraction goes: **a crate owns what is the same for every host; the
host owns what depends on its own runtime, config, or threat model.** Both
crates are therefore synchronous, I/O-free, and runtime-free.

| Crate | Owns | OpenHuman keeps |
| --- | --- | --- |
| `tinydocs` | the `.docx` spec types, their size limits, validation, and OOXML synthesis (`docx-rs` sits behind it) | the artifact pipeline, the `spawn_blocking` hop, and the generation deadline — `src/openhuman/tools/impl/document/` |
| `tinywallet` | the BTC / EVM / Solana / Tron address formats: parsing, validation, encoding conversions | RPC endpoint resolution, transaction assembly and broadcast, key custody — `src/openhuman/web3/` |

Consequences worth knowing before touching either seam:

- **`tinydocs::docx::generate` is synchronous on purpose.** A crate that
  guessed at an executor or a deadline would be wrong for every host that
  guessed differently, so `document/engine.rs` supplies exactly that policy and
  nothing else. `DocumentError::GenerationTimeout` therefore has no `tinydocs`
  equivalent and can only be produced host-side.
- **`tinydocs::Error` is `#[non_exhaustive]`.** The `From` impl in
  `document/types.rs` needs its catch-all arm; it degrades an unmapped variant
  to `GenerationFailed` and logs, so a crate bump that adds a case worth
  handling structurally shows up rather than being swallowed.
- **The JSON tool schema did not change.** `GenerateDocumentInput` is
  `tinydocs`' `DocumentSpec` re-exported under its historical name, with field
  names unchanged; `the_json_wire_shape_is_unchanged_by_the_extraction` pins
  that.
- **`tinywallet` rejects an uppercase `0X` EVM prefix, matching the code it
  replaced, which rejected that prefix too.** The old path went through `ethers_core::types::Address`'s
  `FromStr`, which is `fixed-hash`'s and strips only a lowercase `0x`
  (`fixed-hash-0.8.0/src/hash.rs`, `input.strip_prefix("0x")`), so `0X…` failed
  hex decoding there too. The behaviour is unchanged, verified against the old
  code path rather than assumed — do not "fix" it into leniency.
- **Bitcoin has two rules, not one.** `btc::validate` is the recipient rule;
  `btc::validate_sender` additionally requires P2WPKH. Using the first where
  the second belongs accepts an address that only fails later, at signing time.
- **Each crate's gates ride OpenHuman's existing ones**: `tinydocs` is
  exclusive to `documents`, `tinywallet` to `web3`. Both are default-ON and
  already forwarded to the desktop shell. Note `tinydocs` is now taken with
  `default-features = false` — the wire contract, not the writers, which run in
  the TinyBus module instead (see the module host section).

### Backend API access — `src/api/` over `tinyhumans-sdk`

Calls to the TinyHumans cloud backend go through the vendored
[`tinyhumans-sdk`](https://github.com/tinyhumansai/sdk) crate at
`vendor/tinyhumans-sdk` (git submodule, path dependency — the crate is not on
crates.io, so unlike the other `vendor/` crates it has no `[patch.crates-io]`
entry). **The SDK is the source of truth for backend routes.** A route missing
from it belongs upstream in the SDK repo, not re-implemented in `src/api/`.

The split:

- **SDK** — routes, URL building, percent-encoding, credential headers,
  `{success,data}` envelope handling, and the admin/webhook-receiver route gate.
- **`src/api/`** — the OpenHuman-specific layer on top: session-token retrieval
  (`jwt.rs`), base-URL/env resolution (`config.rs`), and the error
  classification + Sentry policy in `rest.rs`.

`BackendOAuthClient` owns a `TinyHumansClient` built with
`with_http_client(...)` so the SDK inherits this crate's transport — platform
TLS (schannel on Windows for corporate TLS-inspection proxies, rustls
elsewhere), the 120s/15s timeouts, `http1_only`, and the `x-core-version` /
`x-tauri-version` / `x-sdk-name` headers. A session token is bound per call:
`authed_json` does `self.sdk.clone().with_token(Some(jwt))`, so the stored
client stays token-less and concurrent calls with different bearers cannot
race. (`clone()` is Arc-backed — the connection pool is shared, only the token
field differs.)

### Product identity — `x-sdk-name` (`src/api/product.rs`)

OpenHuman, OpenCompany and Medulla share one login and all three reach the
backend through this crate, so every backend-bound request carries
`x-sdk-name` for the backend to attribute it to a product
(`src/utils/sdkSource.ts` in `tinyhumansai/backend`). The value defaults to
`openhuman`; an embedding product overrides it **once during startup, before it
builds any backend client**:

```rust
use openhuman_core::api::{set_product_identity, ProductIdentity};

if let Some(identity) = ProductIdentity::new("opencompany") {
    set_product_identity(identity);
}
```

It is a process-global (`OnceLock<RwLock<_>>`, same shape as
`config::schema::proxy`'s runtime proxy config) rather than a constructor
argument because `BackendOAuthClient::new` is called from ~35 sites across the
domains — none of which a downstream product owns. `BackendOAuthClient` and
`IntegrationClient` read the identity into their default headers when they are
built, so a later `set_product_identity` does not re-tag clients that already
exist — set it during startup, before the first client, and the distinction
never arises. (`MedullaClient` happens to read it per request, but do not rely
on that.)

Five client paths attach it, and each needs its own edit because none shares a
request-building code path with the others:

| Path | Where |
| ---- | ----- |
| `BackendOAuthClient` | both the reqwest transport (`build_backend_reqwest_client`, so `raw_client()` multipart uploads are covered too) and the SDK's `with_default_headers` |
| `IntegrationClient` (`/agent-integrations/*`) | the SDK's `with_default_headers` **only** — its separate `download_client` is deliberately untagged, see below |
| `MedullaClient` | `authed()` for HTTP, and **separately** `sse::StreamState::connect` — the SSE handshake authenticates with a `?token=` query parameter and never reaches `authed()` |
| `desktop::app_state::ops` (`GET /auth/me`) | its local `build_client()` default headers — a hand-rolled TLS client, not `BackendOAuthClient`'s |
| `agent::progress_tracing::langfuse` (`POST /telemetry/langfuse/ingestion`) | at the call site — a bare `reqwest::Client::new()` against the backend's Langfuse proxy route |

**Adding a backend call means adding the header.** The two entries at the
bottom of that table were missed on the first pass and caught in review: both
hand-roll a `reqwest` client against `effective_backend_api_url` with a session
bearer, so neither inherits anything from the three wrapper types above. When
you add a backend-bound request, the question is not "did I use the right
client" but "does *this* request carry `x-sdk-name`". `grep` for
`bearer_authorization_value` and `header(AUTHORIZATION` to find the hand-rolled
ones — those are the paths that go unattributed silently.

`ProductIdentity::new` sanitises with the same allowlist-and-truncate rule
`sanitize_client_version` applies to `x-core-version`, so the wrapped value can
never carry CR/LF and header construction cannot fail.

**Deliberately untagged — do not "fix" these.** `IntegrationClient`'s
`download_client` fetches `/agent-integrations/file-storage/files/{id}/download`,
which answers a 302 to presigned S3. reqwest follows redirects and strips only
*sensitive* headers (Authorization, Cookie, …) when the host changes, so a
custom header like `x-sdk-name` survives onto the storage request; attaching it
per-request does not help, because redirected requests carry the original
headers too. Scoping it to the first hop would mean hand-rolling redirect
following, which is not worth it when every other call in the same session is
already tagged. MCP servers (`mcp::http_client`) and third-party BYOK inference
endpoints are excluded for the same reason: they are not our backend, and
telling an unrelated operator which TinyHumans product a user runs discloses
something for no benefit.

**Not covered** (would need upstream changes, tracked separately): managed
inference and embeddings go out through `tinyagents`' own clients, and the
Socket.IO upgrade sets no HTTP headers at all — its auth rides in the
Socket.IO CONNECT payload. The flow-run Langfuse exporter
(`flows::tinyflows::langfuse_export`) posts to the same
`/telemetry/langfuse/ingestion` proxy as the agent-turn path but goes through
`tinyagents::LangfuseClient`, which builds its own `reqwest::Client` internally
and exposes no seam for default headers or an injected client — so flow traces
stay unattributed until `tinyagents` gains one.

**Every SDK-backed call must map its error through `classify_sdk_error`.** That
function mirrors `authed_json`'s classification exactly (401 →
`Unauthorized`/`SESSION_EXPIRED`, channel-message 404 → `MessageNotFound`,
announcements 404 → `AnnouncementNotFound`, transient statuses logged not
reported). Skipping it would change a route's Sentry and session-expiry
behaviour purely by moving it onto a typed SDK method. `rest_tests.rs` pins the
two paths' equivalence — keep that as call sites migrate.

### Domain layout (`src/openhuman/`)

~31 domain directories — authoritative list: `ls -d src/openhuman/*/`. Major families: agent (`agent` — with `agent/{agentbox,artifacts,context,experience,file_state,harness_init,learning,orchestration,plan_review,profiles,registry,session_db,session_import,tinyagents}`), memory (`memory` — with `memory/{agent,conversations,diff,goals,people,queue,search,sources,store,sync,tinycortex,tool_memory,tree}`), skills/flows (`skills` — with `skills/{catalog,runtime,webhooks}` —, `flows` — with `flows/{tinyflows,rhai}`), inference/AI (`inference` — with `inference/{embeddings,tokenjuice}` —, `routing`), MCP (`mcp` — with `mcp/{server,registry,audit,config_servers,http_client}`), runtimes (`runtime` — with `runtime/{node,python,python_server,pool,javascript}` —, `sandbox` — with `sandbox/cwd_jail`), channels (`channels` — with `channels/whatsapp_data`), meet (`meet` — with `meet/agent`, `meet/backend_bot`), web3 (`web3` — with `web3/{wallet,x402}`), plus kernel domains (`platform` — with `platform/{about_app,connectivity,cost,doctor,health,proc_metrics,service,socket,startup,update}` —, `config` — with `config/{migrations,migration_helpers,workspace}` —, `cron` — with `cron/scheduler_gate` —, `integrations`, `security` — with `security/{approval,credentials,keyring,keyring_consent,encryption,prompt_injection,devices}` —, `threads` — with `threads/{goals,todos}` —, `tools` — with `tools/{registry,status,timeout,agent_policy}` —, `util` — with `util/{text,retry,tls,types}` —, `voice`, …).

**Family directories (in progress).** The flat tree is being collapsed so that **one directory equals one feature gate**: a capability spread across sibling top-level dirs costs a `#[cfg]` per dir plus five parallel registries to keep in sync. Landed so far (124 → 31 top-level dirs, 0 root-level `*.rs`): `meet/`, `util/` (incl. `util/sanitize`), `mcp/{server,registry,audit,config_servers,http_client}`, `sandbox/cwd_jail`, `cron/scheduler_gate`, `runtime/`, `media/`, `voice/audio_toolkit`, `web3/{wallet,x402}`, `medulla/chat`, `flows/{tinyflows,rhai}`, `channels/whatsapp_data`, `desktop/` (accessibility, app_state, dashboard, notifications, overlay, provider_surfaces), `hosted/` (announcements, billing, orchestration, referral, team — all thin proxies to the TinyHumans backend), `subconscious/{triggers,monitors}`, `threads/{goals,todos}`, `tools/{registry,status,timeout,agent_policy}`, `platform/` (about_app, connectivity, cost, doctor, health, proc_metrics, service, socket, startup, update), `config/{migrations,migration_helpers,workspace}`, `integrations/{composio,recall_calendar,file_storage,task_sources}`, `skills/{catalog,runtime,webhooks}`, `inference/{embeddings,tokenjuice}`, `security/{approval,credentials,keyring,keyring_consent,encryption,prompt_injection,devices}` (the kernel security family — never gated), and `agent/{experience,orchestration,registry,agentbox,harness_init,session_db,session_import,context,profiles,learning,plan_review,file_state,artifacts,tinyagents}` (the agent harness is kernel and is never gated; `agent/` stayed put as the parent rather than becoming `agent/core`, which would have cost ~999 extra import rewrites for no gate benefit), and `memory/{store,sync,tree,search,sources,queue,diff,goals,conversations,tool_memory,tinycortex,agent,people}` (the largest family, moved last; `memory/` stayed put as the parent — a `memory → memory/core` rename would have cost ~545 extra rewrites — with the pre-existing `memory/sync.rs` renamed to `memory/sync_events.rs` to free the name for `memory_sync`, and `memory_tools` landing as `memory/tool_memory` to avoid the pre-existing `memory/tools/` agent-tool directory). The `heartbeat/` re-export shim is deleted; use `subconscious::heartbeat` directly. Plan, target tree, and move-PR rules: [`docs/specs/2026-08-02-core-kernel-domain-reorg.md`](docs/specs/2026-08-02-core-kernel-domain-reorg.md).

A move never changes the wire surface — RPC namespaces are string literals in `ControllerSchema`, not derived from module paths — so **do not rename namespace strings to match new paths**.

**Skills runtime**: the QuickJS per-skill VM engine is gone. `src/openhuman/skills/` holds skill metadata/tool descriptors; execution of installed `SKILL.md` workflows lives in `src/openhuman/skills/runtime/` (starts/cancels runs, hosts the `skill_executor` agent, reuses `runtime_node`/`runtime_python`).

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
- **`DomainSet`** selects which *domain families* exist at runtime, one flag per `DomainGroup` (`src/core/all.rs`). Presets: `full()` (default — byte-identical to before #4796), `harness()` (agent + memory + threads + config + security only), `none()`. Every controller is tagged with its `DomainGroup` at the single registration site in `src/core/all.rs`; the live surface (controllers/`/schema`/dispatch, agent tools, stores, subscribers) is filtered by the ambient `CoreContext::domains()`. A gated domain's controllers become unknown-method, its agent tools absent, its stores/subscribers uninitialized. `examples/embed_headless.rs` uses `DomainSet::harness()`; `examples/embed_kernel.rs` uses `DomainSet::kernel()` — the floor (threads + config + security, with `agent`/`memory` OFF) that a host opts subsystems back into by field assignment. Per-gate Cargo `[features]` (children #4797–#4804) narrow the compile-time surface further; `DomainSet` is the runtime axis they compose with.

**`DomainGroup` tracks family directories 1:1.** After the domain reorg (#5328) each variant names a `src/openhuman/` family, so the runtime axis stopped sweeping half the surface into the `Platform` catch-all. Groups: the harness families (`Agent`, `Memory`, `Threads`, `Config`, `Security`), the compile-gate families (`Flows`, `Skills`, `Mcp`, `Meet`, `Channels`, `Web3`, `Voice`, `Media`, `Medulla`), the families carved out of `Platform` (`Inference`, `Integrations`, `Automation` = cron + subconscious, `Runtimes` = runtime + sandbox, `Desktop`, `Hosted`, `Relay` = tinyplace, `Modules` = the native module host), and `Platform` itself — now only the kernel surfaces with no family of their own (`platform/`, `tools/`, `http_host/`, `test_support/`).

That realignment fixed two real defects, both pinned by tests in `src/core/all_tests.rs`:

- `harness()` claimed "agent + memory + threads + config + security" but silently dropped `agent::{agentbox, harness_init, artifacts, learning}`, `security::{credentials, devices}`, `config::{workspace, migration_helpers}`, `memory::people` and `skills::webhooks` into `Platform`. An agent harness that never registers `harness_init` is a latent bug.
- `embedded()` had to set `platform: true` purely to reach credentials and config, which dragged the desktop and hosted-backend surfaces along with it. Those are `Desktop` / `Hosted` now and stay off.

**Adding a family directory means four edits, all compiler-enforced:** the `DomainGroup` variant (`src/core/all.rs`), the `DomainSet` field + `allows()` arm + every preset (`src/core/runtime/builder.rs`).

Three more consumers are *not* compiler-enforced — `tool_group()` (`tools/ops.rs`), `StoreInitPlan` (`runtime/context.rs`) and `DomainSubscriberPlan` (`core/jsonrpc.rs`) — so **drift guards** stand in for the compiler. Each forces every variant into exactly one of two lists (owns-a-store / storeless, registers-subscribers / none, owns-tools / tool-less), so adding a family cannot compile-and-forget:

- `domain_group_all_lists_every_variant` is the root of trust. `DomainGroup::index()` is an exhaustive `match`, so a new variant is a compile error there first; this test then fails until `DomainGroup::ALL` and `COUNT` catch up. The other guards iterate `ALL`, so they are only as good as this one.
- `every_domain_group_is_accounted_for_in_tool_group` tests the *function*, not a built registry — which tools a registry contains depends on config flags, security tier and enabled integrations, so a registry-derived assertion passes or fails for unrelated reasons. `REPRESENTATIVE` holds one real tool name per family; `representative_tool_names_are_real` keeps that table from rotting into dead strings.

These are not theoretical. Two bugs of exactly this shape shipped before the guards existed: `harness_init` sat in `Platform` so `DomainSet::harness()` never registered it, and the `Inference` rule matched `tokenjuice_` while the live tool is `tinyjuice_retrieve` (`tokenjuice_retrieve` is a migration alias), so CCR retrieval leaked to `Platform`. **Match tool names against the owning crate's constants, not a guessed prefix.** A controller whose store keys on a different group than its `push(...)` tag gives you a live RPC surface with no store behind it.

### Compile-time domain gates (Cargo `[features]`)

Per-domain Cargo features drop whole domains **at compile time** (smaller binary, fewer deps), composing with the runtime `DomainSet` axis above.

**There are TWO gate sets, and confusing them is the main hazard here.**

| Set | Where it lives | What it is |
| --- | --- | --- |
| **Contributor** | `[features] default` in `Cargo.toml` | What a bare `cargo check`, `cargo test` and rust-analyzer compile. 9 cheap gates. **353 packages / 3 native builds** (`libsqlite3-sys`, `lzma-sys`, `ring`). |
| **Product** | `scripts/ci/product-features.txt` | What the shipped desktop app has. 16 gates. **540 packages / 7 native builds** (adds `bzip2-sys`, `libgit2-sys`, `libz-sys`, `zstd-sys`). |

`default` used to be the product set, which made the inner loop pay for the whole product on every edit — web3's ethers/secp256k1 cohort, `documents`' zstd/bzip2 native builds (since removed from the graph entirely — the codecs run in a module now), the cpal/hound/arboard/enigo/rdev stack behind `voice`+`inference`, `contacts`' macOS objc2 cohort, `crash-reporting`'s sentry tree, `tui`'s ratatui. Those are default-OFF now. **This did not change what ships**: the shell has set `default-features = false` since #1061 and never inherited `default` anyway.

What it *did* change: **a lane that relies on default features no longer covers the product.** Every CI lane that builds or tests the product passes `--features "$(bash scripts/ci/product-features.sh)"` — clippy, the unit lane, the coverage lane, `scripts/test-rust-with-mock.sh`. If you add a lane, decide which of the two sets it is testing and say so in a comment. Four `tests/*.rs` targets carry `required-features` for the same reason (`json_rpc_e2e`, `raw_coverage_all`, `observability_smoke`, `x402_twit_sh_live`); without those gates cargo **silently skips** them and the run still exits 0 — the same trap `--bins` without `bin-tools` already had.

> **Adding a gate to either set? You must forward it to the desktop shell.**
> `app/src-tauri/Cargo.toml` declares `openhuman_core` with `default-features = false` (set in #1061, before gates existed), so the shipped app does **not** inherit the core's `default` list. A gate in the product set but not in the shell's `features` list is **compiled out of the shipped desktop app** — with no build error and no failing test. This is not hypothetical: `voice` shipped missing from v0.58.19 to v0.61.x (56 users, ~93k Sentry events, #4901), and `tokenjuice-treesitter` was never forwarded once since #4123 and failed *soft*, silently degrading AST compression (#4918).
> `scripts/ci/check-feature-forwarding.mjs` (the **Feature Forwarding Gate** lane) asserts three things: the shell forwards **exactly** `product-features.txt` (set equality, both directions), every name in that file is a real core gate, and every `default` gate is forwarded or allow-listed. The equality check is the load-bearing one — the old subset-of-`default` check would have passed **vacuously** once `default` stopped being the product set, silently re-arming #4901. If a gate genuinely must not ship, add it to `INTENTIONALLY_NOT_FORWARDED` **with a reason** — an explicit exclusion is the only way "deliberate" stays distinguishable from "forgotten".
> A gate in **neither** set (today only `tui`) gets no compile coverage from the normal lanes at all, so the feature-gate-smoke lane checks it explicitly. Put new ones there too.

**Slim-profile convention** (no `full` meta-feature): build slim variants with `cargo build --no-default-features --features "<explicit list of gates you want>"`. This mirrors the existing standalone-feature style (`sandbox-landlock`, `browser-native`, …). Example — everything except voice:

```bash
# check / build without the voice family (incl. audio_toolkit)
GGML_NATIVE=OFF cargo check --manifest-path Cargo.toml \
  --no-default-features
```

#### The kernel profile, and the floor ratchet that protects it

`--no-default-features --features flows` is the **kernel profile**: the surface a
second host would embed to get workflow execution and nothing else. It is measured
and ratcheted, because unmeasured it grows — three heavy dependencies remain
unconditional today (`git2`/vendored-libgit2, `rusqlite`/bundled, and
`tokio-tungstenite`), and none would likely have
landed that way had a number moved in CI when they did.

```bash
scripts/kernel-floor.sh flows        # CI Linux: 304 packages / 281 names / 3 native
scripts/kernel-floor.sh flows --json
scripts/check-kernel-floor.sh        # the CI ratchet (Rust Feature-Gate Smoke lane)
scripts/dep-sim.py --cut-nothing     # calibration: must equal kernel-floor.sh
scripts/dep-sim.py --cut arboard,enigo,rdev   # project a cohort before doing it
```

**CI Linux baseline 2026-08-09: 302 packages / 279 unique names / 2 native
builds** (`libsqlite3-sys`, `ring`). **This is the target** — MIGRATION-PLAN G6
set 2 native builds as the goal, and the profile is there, down from 418 names
/ 6 native when the program started. The four that left: `aws-lc-sys` (the
tinychannels rustls pin), `lzma-sys` (the `runtime-node` gate), and
`libgit2-sys` + `libz-sys` together (the `memory-git` gate). The macOS graph
resolves a few packages higher because of target-specific edges; the CI ratchet
is intentionally calibrated on Linux.

Reaching the target does not retire the ratchet — it is what stops the floor
growing back, and an unmeasured floor grows. `libsqlite3-sys` and `ring` are
both load-bearing (the memory store and TLS), so this is the floor, not a
waypoint.
Limits live in `scripts/kernel-floor.limits`; the ratchet fails on growth **and** on
a shed that was not written back, since an unratcheted improvement grows back
unnoticed.

**Size a cohort with `dep-sim.py`, never by adding up `cargo tree -i` results.**
Per-dependency arithmetic over-counts shared subtrees and misses crates that only
become droppable once a *sibling* is cut — it is how an earlier estimate of ~167
was produced, and that number is wrong. The simulator parses `cargo tree` (not
`cargo metadata`, whose resolve graph is maximal and over-reports by ~36 crates
here, counting dev-dependencies and unenabled target-specific edges), so it agrees
with cargo's feature resolution by construction. CI asserts that calibration.

**49 of 84 direct dependencies contribute zero exclusive crates.** "Make dep X
optional" usually saves nothing on its own — `git2`, `rusqlite`, `reqwest`,
`tokio` and `tokio-tungstenite` have multiple parents. Gate the
whole cohort or expect a delta of 0.

Two columns because there are two sets (see above): **Contrib** is `[features] default`,
**Product** is `scripts/ci/product-features.txt`.

| Feature | Contrib | Product | Gates | Drops deps |
| ------- | ------- | ------- | ----- | ---------- |
| `voice` | OFF | ON | the `openhuman::voice` family (incl. `voice::audio_toolkit`) — STT/TTS providers, dictation server, always-on listening, podcast audio + email | `hound`, `lettre` |
| `inference` | OFF | ON | the `cpal` audio-device stack: microphone capture for voice, plus `desktop::accessibility::permissions`' mic-permission probe. Implied by `voice`. Off ⇒ the probe reports `Unknown`. **The name is historical** — it used to gate the bundled whisper.cpp STT engine, which no longer exists (see the scope note below); do not rename it, it is forwarded by name from the shell manifest and asserted by `INFERENCE_COMPILED_IN` | `cpal` |
| `web3` | OFF | ON | the `openhuman::web3` family (`web3`, `web3::wallet`, `web3::x402`) — crypto wallet (multi-chain sign/broadcast), swaps/bridges/dapp calls, x402 machine payments | `bitcoin`, `curve25519-dalek` |
| `media` | ON | ON | `openhuman::media::generation` (the `media_generate_*` agent tools) + `openhuman::media::image` scaffold | none (surface-only) |
| `documents` | OFF | ON | the `generate_document` / `generate_presentation` agent tools and PDF text extraction during multimodal ingest. **The synthesis is not in this build** — all three run in the `tinydocs` TinyBus module (see below), so this gate turns on the tools and the host policy around them: the artifact pipeline, the deadlines, image resolution under the security policy. `tinydocs` is consumed with `default-features = false`, for the wire contract only. Implies `modules`. Off ⇒ both tools absent from the tool list rather than degraded, and PDF ingest degrades a file to a reference instead of extracted text | **39 crates**, and they leave `Cargo.lock` entirely: `docx-rs`, `ppt-rs`, `pdf-extract` plus `lopdf`, `syntect`, `pulldown-cmark`, `xml-rs`, `quick-xml`, `zip 0.6`, `zstd`, `bzip2`, `encoding_rs`, `euclid`, `ttf-parser`, the CFF/Type1/CMap parsers, … Product profile 505 → 448 names |
| `modules` | ON | ON | `openhuman::modules` — the dynamic module host: the loader that admits a compiled `cdylib` through tinybus's ABI descriptor, manifest, dependency and SHA-256 gates, the compiled-in registry of modules this build trusts, and the `modules` RPC namespace. Implied by `documents`. Off ⇒ `modules.*` is unknown-method and nothing can load a native module | none in the product profile (`ureq`, `flate2`, `tar`, `zip 2`, `tempfile`, `toml` are already there) — **but see the kernel-floor note**: this feature exists so `tinybus/modules` is not enabled on the dependency itself, which would put a `dlopen` loader into the kernel profile where `tinybus` is always-on |
| `meet` | OFF | ON | `openhuman::meet` (join-URL validation) + `openhuman::meet::agent` (live STT/LLM/TTS loop) + `openhuman::meet::backend_bot` (backend-delegated Meet bot over Socket.IO) | none — see note |
| `skills` | ON | ON | `openhuman::skills` + `openhuman::skills::runtime` + `openhuman::skills::catalog` domains — SKILL.md discovery/parse/install, workflow execution + run logs, remote catalogs, the `skill_setup` / `skill_executor` builtin agents, and the 16 skill agent tools | none (see below) |
| `flows` | ON | ON | `openhuman::flows` (saved automation graphs — create/run/schedule, the `workflow_builder` + `flow_discovery` agents), `openhuman::flows::tinyflows` (engine seam), `openhuman::flows::rhai` (`.ragsh` language-workflow tool) | `tinyflows`, `jaq-core`, `jaq-std`, `jaq-json`, `rhai` |
| `mcp` | ON | ON | `openhuman::mcp::server` (the `openhuman mcp` stdio/HTTP server), `openhuman::mcp::registry` (dynamic Smithery installs — `mcp_clients` RPC namespace, SQLite, boot spawn, supervisor, OAuth), `openhuman::mcp::audit` (write-audit log), and the static config-declared server set in `openhuman::mcp::config_servers`. ~19 agent tools, ~20k LOC | **none** (see scope note) |
| `tui` | OFF | — | `openhuman::tui` — the tabbed ratatui/crossterm CLI UI (Logs, Chat, Config, Settings), auto-opened by bare `openhuman` on interactive non-container hosts and forced with `openhuman tui` (alias `chat`). Runs the core in-process. No controllers, no agent tools. **Intentionally NOT forwarded to the desktop shell** (allowlisted in `check-feature-forwarding.mjs`). | `ratatui`, `crossterm` |
| `channels` | ON | ON | `openhuman::channels` (external-messaging providers — Telegram/Discord/Slack/Signal/WhatsApp/iMessage/IRC/… — plus the channel runtime, controllers, host, proactive messaging + inbound dispatch) and the `channels::webview_accounts` / `webview_apis` / `webview_notifications` / `channels::whatsapp_data` webview-bridge domains (incl. the 3 `whatsapp_data_*` agent tools). **Carve-outs `channels::{traits, cli}` stay ungated.** | **28** via `tinychannels/{email,lark}` — the crate itself stays (load-bearing), its two heavy providers do not |
| `memory-git` | OFF | ON | `openhuman::memory::diff` (git-backed snapshots/checkpoints/read markers, the `memory_diff` RPC namespace + agent tool) and the git wiki mirror in `memory::store::content::wiki_git`. **Type carve-out**: `memory::diff::types` compiles in BOTH builds — the always-on subconscious memory profile renders `CrossSourceDiff`/`ChangeKind` into prompts, and tinycortex makes the matching split (its `memory::diff::{types,source}` are ungated, only the `Ledger`/`DiffEngine` half sits behind `git-diff`). Off ⇒ `memory_diff` is unknown-method, the tool is absent, the embedded driver drops `Capability::Diff` **and** `as_diff()` returns `None` in lockstep (`audit_provider` fails on either half alone), and summary nodes are still written to disk but not mirrored into git. | **3**: `git2`, `libgit2-sys`, `libz-sys` — two of the five native C builds in the kernel profile, the largest native shed in the program |
| `contacts` | OFF | ON | `memory::people::address_book`'s macOS CNContactStore reader — the address-book seeding path for the people domain. Leaf gate over a **pre-existing** off-state: the module already shipped a non-macOS `imp` stub returning an empty contact list, so the gate only widens that stub's cfg. `read`/`read_with`/`AddressBookError`/`SystemContactsSource` and the whole `people` RPC surface stay compiled in every build; off ⇒ a refresh seeds nothing instead of failing. | **6** on macOS (`objc2`, `objc2-foundation`, `objc2-contacts`, `block2` + 2 transitive). **No-op on Linux/Windows** — never in those graphs, so the kernel-floor ratchet does not move. Verify cross-target: `cargo tree --target aarch64-apple-darwin -e normal -i objc2-contacts --no-default-features` (294 → 288 packages). |
| `runtime-node` | OFF | ON | `runtime::node` (download / verify / extract / install a pinned Node.js toolchain), the `runtime::javascript` language slot, `runtime::pool::node`, the `node_exec` / `npm_exec` agent tools, and the `node_runtime` harness-init step. **Facade + stub** — `ShellTool` holds `Option<Arc<NodeBootstrap>>` and `shell.rs` is kernel, so the module cannot simply vanish; `runtime/node/stub.rs` carries the `NodeBootstrap` type surface while registration sites are leaf-gated. **The generic native-tool dispatcher (`runtime::node::ops` / `runtime::node::types`) is NOT gated** — it backs both the gated `javascript.*` controllers and the ungated `flows` `oh:` `NativeToolBackend`, so native flow tools (`memory_search`, file, shell, …) keep working when the managed Node runtime is off. Off ⇒ `try_cached`/`probe_installed` return `None` and the shell never prepends a managed bin dir, identical to today's `node.enabled = false` path. | **`xz2` + its static liblzma C build.** First gate to remove a NATIVE toolchain build: `lzma-sys` leaves the list, 6 → 5. `tar`/`zip` are NOT shed — shared with `inference` (install_piper), `runtime::python`, and the document tools. |

**Facade pattern (pathfinder for the other gates).** `pub mod voice;` is **always compiled** as a facade: the real submodules are `#[cfg(feature = "voice")]`, and a `#[cfg(not(feature = "voice"))] mod stub;` (`src/openhuman/voice/stub.rs`) re-exposes the same public surface that always-on / other-gated callers use (`server`, `dictation_listener`, `streaming`, `reply_speech`, `cloud_transcribe`, `cli`, `create_stt_provider`, `effective_stt_provider`, `publish_ptt_transcript_committed`) with no-op / `None` / disabled-error bodies. Callers therefore do **not** need per-call `#[cfg]`. When voice is off: the voice/audio controllers are unregistered (unknown-method over `/rpc`, absent from `/schema`), the `audio_generate_podcast` agent tools are absent, and `openhuman voice` returns a "voice disabled" error. Stub signatures must match the real ones exactly — the disabled build (`--no-default-features`) is the **only** thing that catches drift, so run it before pushing any change to the voice surface.

**Scope note — there is no local STT engine any more.** The bundled whisper.cpp engine (in-process `whisper-rs` plus the `whisper-cli` subprocess fallback), its GGML model/binary downloader (`inference::local::install_whisper` + the `inference.install_whisper` / `inference.whisper_install_status` RPCs), and the `whisper-rs` / `whisper-rs-sys` dependencies were **deleted** from both Cargo worlds. Speech-to-text is now always a hosted HTTP call, and *which* host is a user choice: `voice_server.stt_engine` (`backend` / `elevenlabs` / `openai`) resolved by `voice::factory::effective_stt_provider`, with an explicit `stt_provider` routing string still overriding it. `config::migrations` (9 → 10, `retire_local_whisper_stt`) rewrites a persisted `stt_provider = "whisper"` to `"cloud"`; the factory does **not** silently remap it, so an unmigrated value fails by name instead of hiding.

The `voice` gate still does not drop `llama` or `cpal`: `cpal` belongs to the `inference` gate above, and `llama`/`whisper` inference for the *local model runtime* is a separate concern. Earlier revisions of this note promised a future `inference` gate that would shed whisper — that gate exists and sheds `cpal`; whisper left the graph entirely instead.

**`web3` gate — first gate that sheds real crypto deps.** Same facade pattern: `pub mod wallet;` / `pub mod web3;` / `pub mod x402;` stay always-compiled, real submodules are `#[cfg(feature = "web3")]`, and each domain's `stub.rs` re-exposes the always-on caller surface with disabled-error / empty bodies. When off, the wallet/web3/x402 controllers are unregistered, the web3 swap/bridge/dapp agent tools are absent (via `all_web3_agent_tools()` → empty), and the exclusive `bitcoin` (BTC P2WPKH PSBT) + `ethers-core` / `ethers-signers` / `coins-bip39` (EVM/mnemonic signing, used by the multi-chain wallet's EVM path) deps are dropped. `curve25519-dalek` (used for Solana off-curve ATA here) is **not** among them — it stays enabled transitively through the always-on `ed25519-dalek`. **tinyplace on-chain payments degrade to graceful "wallet disabled" errors** (the tinyplace comms path and the core itself are unaffected — `tinyplace::signer` still works via ed25519). The stubs cover `WALLET_NOT_CONFIGURED_MESSAGE`, `status`, `secret_material`, `WalletChain`, `prepare_transfer`/`execute_prepared` (+ param/result types), `solana_cluster`/`SolanaCluster`/`tinyplace_solana_rpc_endpoints`, `tinyplace_signer_seed`, `wallet::rpc::{redact_rpc_url, with_tinyplace_solana_endpoints}`, and the `all_*_registered_controllers`/`all_*_controller_schemas`/`all_web3_agent_tools` entry points. Two caller families still need per-call `#[cfg(feature = "web3")]` because they name concrete gated types rather than a stubbable aggregator: the six `Wallet*Tool` + `X402RequestTool` registrations in `tools/ops.rs`, the `wallet::tools::*` glob in `tools/mod.rs`, and the x402 402-retry path in `tools/impl/network/http_request.rs` (with the feature off a 402 returns to the caller unpaid).

**`bs58` and `ed25519-dalek` still do NOT drop, deliberately.** `orchestration/ingest` and `tinyplace/payment` use them for agent-network identity, which is unrelated to the wallet. `curve25519-dalek` also survives now, beneath `ed25519-dalek`. Measured: excluding all three from the cohort costs **0**, because tinyplace pulls them in regardless — so there is nothing to gain by chasing them.

`core/all.rs`'s `flows` registration builds a `Vec` and conditionally `push`es rather than using a `vec![]` literal, because an element of a `vec![]` cannot carry `#[cfg]`.

Run the disabled build (`--no-default-features`) before pushing any change to the wallet/web3/x402 surface — it is the only drift catcher. Prove a claimed shed with `scripts/assert-shed.sh`, **not** `cargo tree -i`: the latter exits non-zero when a crate is absent and reports dev-dependency-only survivors as present.

**Leaf-gate variant (`media`, #4804).** Unlike `voice`, the `media` gate needs **no** stub facade: `media::generation` has a single caller (the `build_media_tools` call in `src/openhuman/tools/ops.rs`, itself `#[cfg(feature = "media")]`) and `openhuman::media::image` is unwired scaffold (#2997), so both modules are simply `#[cfg(feature = "media")] pub mod …`. It is a **surface-only** gate: media generation is backend-proxied (`reqwest`, shared) and the `image` crate is shared with channel upload, so no exclusive deps are shed — the issue's "sheds media processing dependencies" / "controllers unregistered" DoD lines are superseded (Media is agent-tools-only; no controller/store/subscriber is tagged `Media`). When a gated domain is a true leaf, prefer this over the facade+stub.
**`meet` gate (#4800)** — the three Meet domains are one **family directory**, `src/openhuman/meet/`:

| Path | Was | Pattern |
| --- | --- | --- |
| `meet/{ops,rpc,schemas,types}` | `meet` | per-submodule `#[cfg(feature = "meet")]` |
| `meet/agent/` | `meet_agent` | leaf-gate — every submodule (incl. `wav`) is `#[cfg(feature = "meet")]` |
| `meet/backend_bot/` | `agent_meetings` | facade + `stub.rs` |

`pub mod meet;` in `src/openhuman/mod.rs` is therefore **ungated**: `backend_bot` is a facade+stub domain, and three always-compiled call sites reach into it — the heartbeat planner (`calendar::handle_calendar_meeting_candidate`) and two subscriber registrations (`core::jsonrpc`, `channels::runtime::startup`) — so `meet/backend_bot/stub.rs` must resolve in a `meet`-less build and those callers need no `#[cfg]`. The gate moved down onto each submodule declaration inside `meet/mod.rs`; the set of items that compiles in each configuration is unchanged.

**RPC namespaces did not move.** They are string literals in `ControllerSchema`, not derived from module paths, so `meet`, `meet_agent`, and `agent_meetings` are still three separate namespaces on `/rpc` and `/schema`, `openhuman.meet_agent_*` method names are untouched, and `DomainEvent::domain()` still reports `"agent_meetings"`. Directory layout and wire surface are independent — do not "fix" the namespace strings to match the new paths.

**No deps to shed (do not re-litigate).** Unlike `voice`, this gate drops **zero** dependencies — the Meet domains have no exclusive crates. `meet::agent::wav` is a hand-rolled 79-line RIFF writer with no `use` statements, written precisely so Meet never needed `hound` (which `voice` already owns and sheds). The dependency shed was pre-paid; this gate's value is compile-time surface and binary size, not the dep tree.


**Both-ways tests.** `src/core/all_tests.rs` pins the gate in both directions (`meet_controllers_registered_when_feature_on` / `meet_controllers_absent_when_feature_off`). The negative half is the one that proves the gate removes anything. Note CI's smoke lane runs `cargo check` only and never compiles test code, so a disabled-build **test** break is invisible to it — run `cargo test --lib --no-default-features core::all::tests` locally after touching any gated surface.

**`skills` gate — the type carve-out (read before adding the next gate).** The three skill domains follow the same facade+stub shape as `voice`, with one important refinement: **`skills` is not a leaf — it is partly load-bearing infrastructure.** `src/openhuman/tools/traits.rs` re-exports the crate's unified `ToolResult` / `ToolContent` out of `skills::types`, and ~236 files consume them (`mcp`, `runtime::node`, every `Tool` impl). `Workflow` / `WorkflowFrontmatter` / `WorkflowScope` from `skills::ops_types` likewise appear in always-on agent-harness and prompt signatures. Gating `skills` wholesale would take down the entire tool trait system, MCP, and the Node runtime.

So `skills::types` and `skills::ops_types` stay **compiled in both directions** — they are inert serde/std-only definitions with zero coupling to their gated siblings — and only *behaviour* is gated. `src/openhuman/skills/stub.rs` therefore mirrors **functions only** and re-exports the real types (`pub use super::ops_types::{Workflow, …}`), so there is **zero type duplication** — strictly less drift surface than the `voice` stub, which had to re-declare `SttResult` + the `SttProvider` trait because those live inside its gated tree.

> **Generalizable rule for the remaining gates:** put a domain's inert types in a dep-free submodule and leave it **ungated**; stub only the behaviour. Reach for a stub type only when the type genuinely cannot be carved out.

Two places the carve-out doesn't reach, and why they are `#[cfg]` at the call site instead of stubbed:

- `agent/registry/agents/loader.rs` — the `skill_setup` / `skill_executor` `BuiltinAgent` entries. `include_str!` embeds the agent TOML from disk regardless of module gating, so the entry itself must disappear.
- `agent/task_dispatcher/executor.rs` — the workflow-resolution branch. `registry::get_workflow` returns `Option<WorkflowDefinition>`, which flattens in `AgentDefinition` and is destructured at the call site; stubbing it would mean re-declaring that struct (exactly what the carve-out avoids). With the domain compiled out no handle can resolve to a skill, so falling through to the builtin-agent branch is correct, not degraded.

**Dep note:** `skills = []` — the empty list is **intentional, do not "fix" it**. Unlike `voice` (`hound`/`lettre`), these domains have no exclusive dependencies: every crate they touch is shared with always-on domains, and `runtime_node` / `runtime_python` are used by Agent / Flows / Memory too. This gate's value is tool-surface + prompt-bloat + startup cost, **not** binary size.

When skills are off: the `skills` / `skill_runtime` / `skill_registry` controllers are unregistered (unknown-method over `/rpc`, absent from `/schema`), the 16 skill agent tools (incl. `run_workflow` / `await_workflow`) are **absent** from the tool list rather than degraded to an error, the `skill_setup` / `skill_executor` builtin agents are gone, and the boot-time remote catalog refresh is skipped. Composes with the runtime `DomainSet::skills` flag (#4796) — that axis needed no change here; #4798 is compile-time only.

**Leaf-gate pattern (`flows`).** Where `voice` needs a stub facade, `flows` needs **none** — and deliberately so. Every symbol reached from outside the gate is a *registration site* (controller push in `src/core/all.rs`, the `FlowTriggerSubscriber` in `src/core/jsonrpc.rs`, boot reconcile in `src/core/runtime/services.rs`, agent-tool `vec!` elements in `src/openhuman/tools/ops.rs`, `BuiltinAgent` entries in `agent/registry/agents/loader.rs`). Registration sites want **absence**: a stub that registered a controller returning `Err("flows disabled")` would make `flows.*` a *known* method that fails at runtime — the opposite of the intended "unknown method / omitted tool". So the family carries a **single** `#[cfg(feature = "flows")]` on `pub mod flows;` in `src/openhuman/mod.rs` — the nested `flows::tinyflows` and `flows::rhai` submodules inherit it — and each call site carries its own `#[cfg]`. The leaf gate holds only because no always-compiled domain has a real code edge into the tree: `memory/tools.rs` and `memory/tools/flavour.rs` name `flows::tinyflows` in comments only. There is no `openhuman flows` CLI subcommand, so no CLI stub is needed either. When flows is off: the `flows.*` controllers are unregistered (unknown-method over `/rpc`, absent from `/schema`), all 25 flow agent tools + the `rhai_workflows` tool are absent, and the `workflow_builder` / `flow_discovery` built-in agents are not advertised.

**Scope note (`flows` deps):** the gate sheds `tinyflows` + its `jaq-core` / `jaq-std` / `jaq-json` JSON-query stack, and `rhai`. It does **not** shed `tinyagents` — 26+ domains consume that crate. The issue-level DoD line reading "sheds the rhai scripting engine" is therefore true only at the **feature** level: `rhai` arrives via `tinyagents/repl`, which the root `Cargo.toml` no longer enables directly — the `flows` feature turns it on. Dropping `flows` drops `repl`, which drops `rhai`; `tinyagents` itself stays. Verify a claimed shed with `cargo tree -i <crate> --no-default-features` (must return nothing) — compiling clean is **not** proof that a dep was dropped.

**Testing gotcha (applies to every gate).** The CI smoke lane runs `cargo check` only — it never runs `cargo test --no-default-features`, so CI stays green while the disabled-build **test** suite is broken. Tests that hard-assert a gated family (`.expect("a flows.* method exists")`, `assert!(full_ns.contains("flows"))`, `group_for_namespace("flows")`, built-in-agent id lists) must be `#[cfg]`-gated in lockstep with the feature. Run `GGML_NATIVE=OFF cargo test --lib --no-default-features core::` locally before pushing any gate change.

#### The `mcp` gate

Follows the voice facade+stub pattern for `mcp::server` / `mcp::registry` / `mcp::audit` (`stub.rs` in each), with two refinements worth copying:

- **The family root `pub mod mcp;` is UNGATED.** It cannot carry `#[cfg(feature = "mcp")]` for two independent reasons: `mcp::http_client` is always compiled (below), and the three facades each ship a `stub.rs` that must resolve in an `mcp`-less build. The gate is pushed down onto each member in `src/openhuman/mcp/mod.rs` — the same rule the `meet/` pilot proved. `mcp::config_servers` is leaf-gated there; `mcp::http_client` is not gated at all.

- **Type carve-out.** Inert, dependency-free type modules stay **ungated**: `mcp::registry::types`, `mcp::audit::types`, `mcp::server::tools::types` (`McpToolSpec`). They are `serde`/`serde_json`-only data consumed by always-compiled callers (the orchestrator prompt builder, `tool_registry`). Both builds therefore share the **one real type definition** — the stubs carry behaviour only, so struct fields can never drift between the enabled and disabled builds. `ConnectedServerOverview` was moved from `connections.rs` into `types.rs` for exactly this reason and is re-exported from `connections` so existing paths still resolve.
- **Split facade — the old `mcp_client` directory did not match the dependency graph, so the reorg split it three ways.** Its transport primitives went to the **ungated** `mcp::http_client` (`McpHttpClient`, `redact_endpoint`, `McpUnauthorizedError`); its static server set + stdio transport + setup agent went to the **leaf-gated** `mcp::config_servers`; and `sanitize` left the family entirely for `util::sanitize`. The `gitbooks` docs tool dials `McpHttpClient` directly (GitBook is modelled as a legacy MCP server), and the orchestrator prompt sanitizes **skill** descriptions through `util::sanitize::sanitize_for_llm` — neither has anything to do with MCP, and stubbing them would silently break a docs tool and corrupt the orchestrator prompt in slim builds. **The gate follows the real dependency graph, not the directory name.** A bonus of keeping `http_client` compiled: the `McpServerNeedsAuth` classifier coupling test in `core::observability` stays always-compiled — no `#[cfg]`, no wording-drift leak.

**Scope note — the `mcp` gate drops ZERO dependencies.** There is no MCP SDK in this crate: the dependency declarations contain no MCP-specific SDK or transport dependency; `test-mcp-stub` is the only MCP-named bin target. The entire protocol stack is hand-rolled over tokio process stdio + `reqwest` + `axum`, all of which are load-bearing for non-MCP domains. The gate is worth having for the ~20k LOC / ~19 agent tools / RPC surface it removes, but the issue-level DoD line claiming it "sheds the MCP SDK / transport stack" is superseded by this correction. The `mcp = []` feature list in `Cargo.toml` is intentionally empty — do not "fix" it by adding `dep:` entries.

**Static vs dynamic — the naming is INVERTED from intuition.** Both halves must be gated or the gate is only half-applied:

| Module | Despite the name, it is… | Backed by | Agent tools |
| ------ | ------------------------ | --------- | ----------- |
| `mcp::config_servers` | the **STATIC**, config-declared server set (`[[mcp_client.servers]]` in TOML → `McpServerRegistry::from_config`) | TOML config | `mcp_list_servers`, `mcp_list_tools`, `mcp_call_tool` |
| `mcp::registry` | the **DYNAMIC**, user-installed Smithery servers (live connection map, boot spawn, supervisor, OAuth) | SQLite `mcp_clients.db` | 11 × `mcp_registry_*` |

**CLI when compiled out.** `src/core/cli.rs` is deliberately **untouched**: the `"mcp" | "mcp-server"` arm resolves to the stub's `run_stdio_from_cli`, which returns a "mcp feature disabled at compile time … rebuild with `--features mcp`" error. Deleting the arm would let `mcp` fall through to generic namespace resolution and fail with `unknown namespace: mcp` — which reads like a user typo rather than a build fact, and would leave an MCP host (Claude Desktop / Cursor) hanging on stdout that never speaks JSON-RPC. Pinned by `mcp_subcommand_reports_disabled_build_when_gate_off` in `src/core/cli_tests.rs`.

**Dangling `mcp_agent` in the orchestrator TOML is expected and safe.** `agent.toml` is data and cannot be `#[cfg]`'d, so the orchestrator keeps listing `mcp_agent` in `subagents` even when the agent is compiled out. Both resolution sites already tolerate unknown ids — `collect_orchestrator_tools` warns and skips, `validate_tier_hierarchy` `continue`s — so the core still boots. `orchestrator_tolerates_unresolvable_subagent_id` / `orchestrator_tolerates_absent_mcp_agent` in `loader.rs` pin that contract; do not "tighten" unknown-subagent handling into a hard error without re-checking them. `src/core/legacy_aliases.rs`'s frontend-catalog drift tests ignore gated namespaces for the same data-vs-code reason.

`src/core/all.rs` needs **no** `#[cfg]` for this gate: the stub aggregators return empty vecs, so the registration sites keep compiling unchanged.

### Loadable native modules — `src/openhuman/modules/`

A capability can live outside this binary. A module is a compiled `cdylib`
speaking the tinybus module ABI: downloaded from a pinned release, verified
against a digest compiled into `modules::registry`, admitted through tinybus's
ABI and manifest gates, and attached to a private in-process broker as an
ordinary bus peer. The core then calls it over that bus like any other service.
`documents` is the first consumer — `.docx` / `.pptx` synthesis and PDF
extraction all happen in the `tinydocs` module.

**What it buys is a dependency boundary that survives compilation.** A codec is
not kernel work, and each one drags a tree of parsers into a binary that mostly
does something else. Moving one out removes its dependencies from the build
rather than merely gating them: `documents` went from 39 crates to none.

**What it costs is process isolation, and that is not small.** A loaded module
shares this address space, these privileges and this crash domain; tinybus's
deadlines, bounded queues and caught panics contain ordinary misbehaviour, not a
segfault. `dlopen` runs code before any symbol can be inspected, so the ABI,
manifest and digest gates decide what is **admitted**, never what is **safe**.
Modules are first-party code that ships separately. Anything untrusted belongs in
a process.

**tinybus never unloads a library.** A module that is refused or faulted is
failed until the process restarts, which is why `modules::ops` caches failures
instead of retrying — the alternative is paying a download and a `dlopen` per
tool call to reach the same error.

Five decisions worth knowing before touching this:

- **The registry is a compiled-in `const` table.** Which modules exist, which
  interfaces they claim, and which bytes are legitimate are build-time decisions.
  Neither config nor RPC can name an artifact: a registry a server could add
  entries to would be remote code execution with a download step. `[modules]`
  config controls only whether modules load, whether this host may fetch them,
  and where a developer's own build lives.
- **Digests are pinned in source as the host's half of a two-sided check.**
  tinybus fetches the release's own `checksum.toml`, compares it with ours,
  hashes the download, and extracts only after. Pinning here makes the check
  auditable offline and makes a release re-cut under the same tag stop matching
  rather than silently replacing what runs in-process. Take the values verbatim
  from the release; never recompute them from a local build.
- **Artifact selection returns an ordered list, not one answer.** A target triple
  is not enough — a `.so` built against glibc 2.39 fails to `dlopen` on a 2.35
  host with a symbol-version error the ABI gate cannot phrase helpfully. So
  releases publish per-distro artifacts, `modules::platform` probes glibc, prefers
  the newest build that could work, and falls through on admission failure. A musl
  or BSD host gets an empty list: "unsupported" beats a download that cannot load.
- **Admission is permissive, deliberately.** Strict mode additionally refuses a
  module whose rustc version differs from the host's, and the real published
  artifact **is** refused that way — released artifacts are built on whatever
  toolchain CI had and this crate pins its own, so mismatch is the normal case.
  Strict mode would have meant the feature never worked in the field while every
  local build looked fine. Everything protecting the address space is still
  enforced; only the toolchain string is relaxed.
- **Modules run on their own broker**, because `OnceBus::init_in_process` builds
  its `Broker` privately and `ModuleHost::new` needs one. The consequence: a
  module cannot publish a `DomainEvent`. Fine for a codec; revisit if a module
  ever needs to emit events.

**The bus belongs to whichever runtime creates it.** In the core that is the one
runtime the process has. In tests it is not: two `#[tokio::test]` functions each
build their own, and the second to call a loaded module finds a broker whose tasks
died with the first — the call **hangs** until some deadline above it fires. Any
test driving a real module must be the only one in its process, which is why the
module-backed tool tests are `#[ignore]`d rather than merely gated on an artifact.
Run them one at a time with `OPENHUMAN_MODULE_PATH` pointing at a directory
holding the built library.

**Payloads in and out are not symmetric.** Inbound bytes ride a tinybus stream
opened alongside the call, so flow control and the size cap are the bus's. Replies
cannot: `Interface::call` receives no caller identity and no connection, so a
served object cannot open a stream back to its caller. A produced document is held
by the module and pulled in chunks. A reply-stream seam upstream would remove that
half.

**`modules` must not be enabled on the tinybus dependency directly.** `tinybus` is
always-on kernel surface, so `features = ["modules"]` there puts a loader plus
`ureq` and an archive stack into the kernel profile for a host that can never use
one — 305 → 308 packages, which the kernel-floor ratchet caught. It is forwarded
from this crate's own `modules` feature instead.

#### The `tui` gate

The tabbed terminal UI (`openhuman`, or explicitly `openhuman tui` / alias `chat`) lives in `src/openhuman/tui/` and follows the **`mcp`/`voice` facade+stub** pattern: `pub mod tui;` is always compiled; the behavioural submodules (`app`, `render`, `state`, `terminal`, `runner`) are `#[cfg(feature = "tui")]`; and `#[cfg(not(feature = "tui"))] mod stub;` re-exposes the one symbol an always-compiled caller reaches — `run_from_cli` — with a build-fact error body (`"tui feature disabled at compile time … --features tui"`). Bare-command auto-launch requires terminal stdin/stdout and `HostKind::Cli`; Docker, CI, pipes, and `--no-tui` retain the non-TUI CLI path.

- **The `"tui" | "chat"` CLI arm in `src/core/cli.rs` is un-`#[cfg]`'d on purpose.** In a slim build it resolves to `tui::stub::run_from_cli`, which bails with the disabled-error rather than falling through to `unknown namespace: tui` (which reads like a typo, not a build fact). Same reasoning as the `mcp` arm. Pinned by `tui_subcommand_reports_disabled_build_when_gate_off` / `chat_alias_reports_disabled_build_when_gate_off` in `src/core/cli_tests.rs` (both `#[cfg(not(feature = "tui"))]`). `"tui" | "chat"` is also added to the banner-suppression `matches!` (a TUI owns the terminal — a banner would corrupt it).
- **No controllers, no agent tools, no `all.rs` changes.** The TUI is a pure *client* of existing registered controllers — it boots the core in-process (`CoreBuilder::new(HostKind::detect_standalone()).domains(DomainSet::full()).services(ServiceSet::none())`), sends chat turns through `web_chat`, reads a bounded in-memory copy of the file-only core log stream, edits only curated safe config getters/updaters, and invokes auth controllers for account/status actions. Never render `config.get` wholesale because the full snapshot can contain secrets.
- **Terminal hygiene is load-bearing.** `logging::init_for_tui` installs a **file-only** subscriber (never stderr) — a single core boot log on stdout/stderr would corrupt the alternate-screen UI. `terminal::TerminalGuard` restores raw mode + the main screen on `Drop`, and a panic hook chains a restore ahead of the default hook. All `[tui]` state-transition logs go to the file, never `println!`.
- **Intentionally NOT forwarded to the desktop shell** (the app ships its own Tauri UI). It carries the only current entry in `INTENTIONALLY_NOT_FORWARDED` in `scripts/ci/check-feature-forwarding.mjs`; the pure reducer lives in `src/openhuman/tui/state.rs` (`TranscriptState::apply_event`) with unit tests, so most behaviour is testable without a terminal.

Drops the exclusive `ratatui` + `crossterm` deps when off. Verify with `cargo tree -i ratatui --no-default-features` (must return nothing).
#### The `channels` gate (#4801 — last child of #4795)

Leaf-gate pattern with **two ungated carve-outs and no stub file** — the reach-map put every gated symbol at a *registration/leaf* call site, so absence (unknown-method / omitted tool), not a disabled-error stub, is the correct off-state (same rationale as `flows` / `meet`).

- **Now sheds 28 crates** — `channels = ["tinychannels/email", "tinychannels/lark"]`. This bullet previously read "Sheds ZERO dependencies — do NOT re-litigate", and the premise behind it is still true and still worth knowing: **`tinychannels` itself can never be gated out.** `config/schema/channels.rs` re-exports its config types, `event_bus/events.rs`'s `DomainEvent` embeds `tinychannels::ChannelInboundEnvelope` in an always-on enum, and `security/pairing.rs` re-exports its pairing helpers.

  What was wrong was the conclusion, not the premise. The heavy crates do not belong to *tinychannels*, they belong to two of its **providers** — `providers::email_channel` (lettre + async-imap + mail-parser, 18 crates) and `providers::lark` (axum + prost, 9). Both are exclusively reachable through it, so gating them **inside the vendored crate** sheds them while the envelope, config, and pairing types stay compiled. Nothing needed stubbing.

  That mattered: gating the crate out would have required stubbing ~28 items, among them `constant_time_eq`/`hash_token` (a wrong stub is a security bug) and `build_session_key_for_inbound_envelope`, which derives a **persisted** conversation key that `memory_conversations/bus.rs` writes — silent data regrouping if it ever drifted. Gate the providers, never the crate.

  Two couplings to keep in mind when touching this: **`voice` also requires `tinychannels/email`**, because `voice::audio_toolkit::ops` delivers generated podcasts through `EmailChannel` — a voice-enabled, channels-less build still needs the provider. And `providers/discord/api_tests.rs` uses `axum` for a mock server unrelated to Lark, so axum is dual-declared as a dev-dependency in tinychannels and must stay that way.

  (`whatsapp-web` is a **refinement inside** the gate — `whatsapp-web = ["channels", "tinychannels/whatsapp-web"]`.)
- **Two ungated carve-outs.** `pub mod traits;` (a one-line `tinychannels` `Channel`/`SendMessage` re-export) and `pub mod cli;` (`CliChannel`, a dependency-free local stdin/stdout REPL) stay compiled in **all** builds — both are reached by the always-on agent-harness interactive loop (`agent::harness::session::runtime::run_interactive`). Same shape as the `meet::agent::wav` carve-out. `channels::mod.rs` `#[cfg(feature = "channels")]`s everything else; nothing inside the gated submodules changes.
- **The in-app web chat is NOT gated.** `openhuman::web_chat` (RPC namespace `channel`, decoupled from `channels/` in #5002 + #5003 which also moved `learning` out) is core product surface and stays always-compiled even though its runtime tag is `DomainGroup::Channels`. Its registration push in `src/core/all.rs` is deliberately left ungated; the both-ways test pins `channel` present with the feature OFF.
- **Three mis-housed imports were retargeted to `tinychannels` (no stub needed).** `cron/bus.rs` (`Channel`/`SendMessage`/`ChannelMessage`), `memory_conversations/bus.rs` (`ChannelMessage` + `context::conversation_history_key`), and `voice/audio_toolkit/ops.rs` (`providers::email_channel::EmailChannel`) reached the gated domain only to pick up symbols that actually live in `tinychannels`; pointing them straight at the crate removes the always-on → gated edge (and the voice→channels cross-gate edge). The old `channels::` paths were 1-line delegations / `pub use` re-exports of exactly these.
- **Leaf-gated call sites** (each carries its own `#[cfg]`): the 5 controller-registration pushes in `src/core/all.rs` (channels controllers, `webview_apis`, `webview_notifications`, public + internal `whatsapp_data`), the `ChannelInboundSubscriber` + web-only-proactive block in `src/core/jsonrpc.rs`, `spawn_channels_service` in `src/core/runtime/services.rs`, the `whatsapp_data::global::init` block in `src/core/runtime/context.rs`, and the `whatsapp_data::tools::*` glob + 3 `WhatsAppData*Tool` registrations in `src/openhuman/tools/{mod,ops}.rs`. The `whatsapp_data` `pub mod` declaration now lives in `channels/mod.rs` (still `#[cfg(feature = "channels")]`, because the parent stays ungated for the `traits`/`cli` carve-outs); `webview_apis` / `webview_notifications` moved under `desktop/` in the family reorg and stay leaf-gated there. String-match arms (`"channels" =>` descriptions, `whatsapp_data_` in `group_for_namespace`) stay **ungated** — they are data.
- **`start_bootstrap_jobs`' `services.channels` block keeps running slim** — it drives composio sync / workspace-memory sync / orchestration drain and names **no** `channels::` symbol, so it stays ungated by design.
- **No CLI change.** There is no `openhuman channels` subcommand; generic namespace resolution yields "unknown namespace" when off (the `flows` precedent — acceptable).
- **Both-ways tests.** `channels_controllers_{registered_when_feature_on,absent_when_feature_off}` in `src/core/all_tests.rs` pin the controller surface (the OFF half also asserts `channel`/web_chat survives), and `whatsapp_data_tools_{present_when_channels_on,absent_when_channels_off}` in `src/openhuman/tools/ops_tests.rs` pin the 3 agent tools (that module has the full-tool-list machinery). CI's smoke lane runs `cargo check` only, so run `cargo test --lib --no-default-features core::all::tests` locally after touching any gated surface.

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
- **i18n**: all UI text through `useT()` from `app/src/lib/i18n/I18nContext`. Add each key to `en.ts` **and real translations to every locale file** (`ar`, `bn`, `de`, `es`, `fr`, `hi`, `id`, `it`, `ko`, `pl`, `pt`, `ru`, `zh-CN`), preserving interpolation placeholders exactly. Translation values must not contain em dashes (`U+2014`); use natural, locale-appropriate punctuation and phrasing, never literal or machine-sounding copy. Run `pnpm i18n:check`, `pnpm i18n:english:check`, and the i18n coverage test before submitting changes.
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

Update `src/openhuman/platform/about_app/` when adding/removing/renaming user-facing features. Define E2E scenarios up front covering happy paths, failures, auth gates.

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
