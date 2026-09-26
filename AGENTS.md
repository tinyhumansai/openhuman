# OpenHuman

OpenHuman is a React and Tauri v2 desktop assistant with an in-process Rust
core. The core also exposes JSON-RPC and a CLI.

Architecture: [overview](gitbooks/developing/architecture.md),
[frontend](gitbooks/developing/architecture/frontend.md),
[Tauri shell](gitbooks/developing/architecture/tauri-shell.md), and
[agent harness](gitbooks/developing/architecture/agent-harness.md).

## Repository map

| Path | Purpose |
| --- | --- |
| `app/src/` | Vite and React frontend |
| `crates/openhuman-app/` | Thin desktop host; excluded from the root workspace, build with `--manifest-path crates/openhuman-app/Cargo.toml` |
| `crates/openhuman-core/` | Package `openhuman`: business domains under `src/<domain>/`, transport/dispatch/auth under `src/core/` |
| `crates/openhuman-core/src/<domain>/` | Flat business-domain modules (agent, memory, tools, security, channels, ...) |
| `crates/openhuman-core/src/core/` | CLI, JSON-RPC and HTTP dispatch, controller registry, event bus, runtime composition; no business logic |
| `crates/openhuman-cli/` | The `openhuman-core` binary (`src/main.rs`), the developer/benchmark bins (`src/bin/`), and every root `tests/*.rs` / `examples/*.rs` target; depends on `openhuman-tinyhumans` for the backend transport the core does not carry |
| `crates/openhuman-embed/` | Typed library facade for embedding the core in another product |
| `crates/openhuman-rpc/` | Shared RPC contracts, response decoding, and HTTP client used by app and TUI |
| `crates/openhuman-tinyhumans/` | The TinyHumans layer above embed: SDK-backed backend transport, a `RuntimeBuilder` that boots connected, and the host-side login/session owner (login-token exchange, `/auth/me`, current-user cache, credential handoff) used by app and TUI |
| `crates/openhuman-tui/` | Standalone terminal frontend |
| `tests/` | Rust integration and JSON-RPC tests |
| `gitbooks/` | Public product and contributor documentation |
| `docs/` | Internal maintainer documentation |
| `vendor/` | Recursive git submodules; root `Cargo.toml` `[patch]` tables point into this tree |

Run commands from the repository root. The root package is a private pnpm
workspace.

## Product boundaries

- The shipped Tauri product targets Windows, macOS, and Linux.
- The experimental iOS client is not part of the shipped desktop host. Its
  transport implementations live in `app/src/services/transport/`.
- The Rust core owns business rules, persistence, execution, RPC, and CLI
  behavior.
- The frontend and Tauri shell present or orchestrate core behavior. Do not
  duplicate core policy in TypeScript or shell code.
- The desktop core runs as a tokio task managed by
  `crates/openhuman-app/src/core_process.rs`. Frontend RPC uses the per-launch bearer
  returned through the `core_rpc_token` command.
- `OPENHUMAN_CORE_REUSE_EXISTING=1` connects the shell to an external core for
  debugging.

## Common commands

```bash
pnpm install
pnpm dev
pnpm dev:app
pnpm build
pnpm typecheck
pnpm lint
pnpm format
pnpm format:check
pnpm test
pnpm test:coverage
pnpm test:rust

cargo check --manifest-path Cargo.toml
cargo build --manifest-path Cargo.toml -p openhuman-cli --bin openhuman-core
cargo check --manifest-path crates/openhuman-app/Cargo.toml

# Standard root-crate validation
cargo check --manifest-path Cargo.toml
```

Use the summary-sized debug runners for long test output:

```bash
pnpm debug unit [test-file]
pnpm debug unit -t "test name"
pnpm debug e2e [spec]
pnpm debug rust [filter]
pnpm debug logs last
```

Debugging the web UI in a real browser (Chrome DevTools MCP): the desktop
window is Wry (WKWebView / WebView2 / WebKitGTK), which does not support CDP, so no
DevTools client can attach to it. Run the same SPA in Chrome instead:

- **Use the desktop app's session:** `pnpm dev:app:web:attach -- --no-browser`
  finds the running desktop core, starts Vite, and prints
  `http://127.0.0.1:<core>/dev/connect?app=http://localhost:<vite>`. Open that
  URL with the chrome-devtools MCP (`new_page` / `navigate_page`). The core's
  dev-only `GET /dev/connect` (`crates/openhuman-core/src/core/dev_connect.rs`)
  redirects to Vite's `/__dev-connect` page with the RPC URL and bearer in the
  URL fragment. That page seeds them into `localStorage`, so the browser runs on
  the desktop core with its signed-in user, and nothing is pasted. The route
  exists in debug builds, or in a release build with `OPENHUMAN_DEV_CONNECT=1`.
  It accepts only a loopback `app` origin and `Host`, and refuses cross-site
  navigations.
- **Fresh core instead:** `pnpm dev:app:web --no-browser` builds and starts its
  own `openhuman-core serve` with a generated bearer and prints
  `http://localhost:<vite>/__dev-connect`. Sign-in takes one click on a provider:
  the browser build passes `<origin>/__dev-auth` as the backend `redirectUri`,
  and the session persists in that core's workspace.
- Busy ports are fine. Vite moves to the next free port, and so does
  `openhuman-core serve`, even when another live core holds the port.
  (`OccupiedByCore::Fallback`; only the desktop shell's embedded core runs the
  stale-listener takeover.) The printed URL always uses the real ports.
- Onboarding and the walkthrough tour are skipped by default
  (`VITE_DEV_SKIP_ONBOARDING`, marked complete in the core for a signed-in
  user). Pass `--onboarding` to keep them for debugging.
- Inspect with `take_snapshot`, `list_console_messages`,
  `list_network_requests` (check the `/rpc` calls), `evaluate_script`, and
  `take_screenshot`.
- Env settings: `OPENHUMAN_DEV_PORT`, `OPENHUMAN_CORE_PORT` (in attach mode, the
  desktop core's port; the default scans 7788-7808), `OPENHUMAN_CORE_TOKEN`, and
  `OPENHUMAN_WORKSPACE` (point it at a scratch dir for a clean profile).

Long CI build or test commands must run through
`scripts/ci-cancel-aware.sh`. Do not export `CARGO_TARGET_DIR`; the repository
already configures shared build output where appropriate.

Keep matching profile settings synchronized between `Cargo.toml` and
`crates/openhuman-app/Cargo.toml`:

- Development dependencies use `debug = false`.
- Release builds use thin LTO, one codegen unit, symbol stripping, and
  `debug = "line-tables-only"`.

## Testing and CI

CI Lite runs area-specific checks and changed-line coverage on PRs to `main`
or `release`. CI Full runs the complete suites for `release`. Changed-line
coverage must be at least 80 percent.

- Frontend unit tests are colocated as `*.test.ts` or `*.test.tsx` under
  `app/src/`. Use Vitest and test behavior rather than implementation.
- Rust domain tests live beside their modules. Use
  `scripts/test-rust-with-mock.sh` for tests that need the shared mock backend.
- JSON-RPC behavior belongs in Rust E2E tests, commonly
  `tests/json_rpc_e2e.rs`.
- Frontend flows need mocked browser or desktop E2E coverage under
  `app/test/e2e/specs/`.
- E2E code must use `app/test/e2e/helpers/element-helpers.ts`, not raw platform
  element types.
- Tests must not call real backend or third-party services.
- Avoid time-based flakes and real network access in unit tests.
- Root `tests/*.rs` and `examples/*.rs` are targets of **`crates/openhuman-cli`**
  (the core is a library and declares no bin/test/example targets). They are
  NOT auto-discovered (`autotests = false`, `autoexamples = false`); every new
  file needs an explicit `[[test]]` / `[[example]]` entry in
  `crates/openhuman-cli/Cargo.toml` with `path = "../../tests/<name>.rs"`;
  `pnpm rust:layout` (`scripts/ci/check-openhuman-rust-layout.mjs`) fails on
  missing or stale entries, on any target table in the core manifest, on
  inline `#[cfg(test)] mod` blocks, and on files named `tests.rs`/`test.rs`.
  Files under `tests/raw_coverage/` are aggregated by the root `build.rs`
  (`build = "../../build.rs"`) into the single `raw_coverage_all` target and
  need no entry. Run them as `cargo test -p openhuman-cli --test <name>`.
- A suite that boots the core **in-process** and reaches the backend (mock)
  must call `tinyhumans_boot::boot()` from `tests/support/tinyhumans_boot.rs`
  first; the core has no backend transport of its own, and without it every
  backend call answers `BACKEND_UNAVAILABLE:`. Suites that spawn the
  `openhuman-core` binary get it from `main.rs`.

Shared mock backend:

- Core routes: `scripts/mock-api-core.mjs`
- Server: `scripts/mock-api-server.mjs`
- E2E adapter: `app/test/e2e/mock-server.ts`
- Manual start: `pnpm mock:api`

Debugging what the core actually sends to inference (prompt size, tool
schemas, cache keys, which endpoint answered, time to first byte, cached
tokens): put the capture proxy between the core and its backend instead of
guessing from logs.

- `CAPTURE_ALL=1 pnpm debug capture` (`scripts/debug/capture-first-inference.mjs`)
  listens on `127.0.0.1:18765`, forwards everything to `CAPTURE_UPSTREAM`
  (default `https://api.tinyhumans.ai`; `https://openrouter.ai` for a direct
  BYOK route), dumps every inference request body under
  `target/debug-logs/inference-sequence/`, and prints one line per response:
  `served_by`, `ttfb`, `prompt`, `cached`, `cache_key`, status, error.
- Point a core at it with `api_url = "http://127.0.0.1:18765"` in the user
  `config.toml` or `BACKEND_URL=http://127.0.0.1:18765` on a headless
  `openhuman-core run`; drive turns over JSON-RPC (`channel_web_chat`).
- Read the lines as claims to check: `cache_key` must be identical across the
  turns of one thread, `served_by` should not change mid-thread, and `cached`
  should approach `prompt` from the second call on. Any of those drifting is
  the finding.
- Self-test: `scripts/__tests__/capture-first-inference.test.mjs` (runs in the
  CI scripts lane). The static prompt on its own comes from
  `openhuman-core agent dump-prompt --agent <id> --json --with-tools`
  (`scripts/debug-agent-prompts.sh`); the proxy shows the request the harness
  assembles from it per turn.

## Configuration and security

- Copy environment settings from `.env.example` and `app/.env.example`.
- Frontend environment access is centralized in `app/src/utils/config.ts`.
  Do not read `import.meta.env` elsewhere.
- Rust configuration is defined under
  `crates/openhuman-core/src/config/schema/` and loaded through its config operations.

The autonomy policy is **off by default** (`[autonomy] enabled = false`,
`config/schema/autonomy.rs`). Agents here run inside containers, platform jails
and Docker sandboxes that already provide the isolation this in-process policy
was approximating, and a shell tool that refuses ordinary shell syntax is not a
usable shell. `SecurityPolicy::from_config` carries the flag; every enforcement
entry point short-circuits on it.

With the policy **disabled** (the default):

- Command classification, the approval gate, the command allowlist and the
  hourly action budget are all inert.
- `workspace_only`, `forbidden_paths` and the workspace-internal boundary are
  not enforced.
- `is_always_forbidden` still is: credential stores (`~/.ssh`, `~/.gnupg`,
  `~/.aws`) and system roots stay unreachable. So do `..` traversal and null
  bytes in a path. Keep that floor.

With `[autonomy] enabled = true`:

- `action_dir` is the agent's permitted read and write root — and is granted as
  a `ReadWrite` trusted root by `from_config`, so changing the working folder
  does not cost the agent write access to it.
- `workspace_dir` stores internal state and is never an acting-tool target.
- Unknown commands classify as writes.
- The approval gate prompts on non-read classes. Interactive requests expire as
  denied after ten minutes.

Either way, sandboxed agents use the platform jail or Docker backend, and the
Rust path checks still apply if the sandbox falls back.

A quoted heredoc body (`<< 'EOF' … EOF`) is **data**, not shell:
`strip_quoted_heredoc_bodies` blanks it before any structural scan. An unquoted
delimiter (`<< EOF`) is still expanded and still scanned.

## Frontend

The provider chain is documented and generated from `app/src/App.tsx`.
Update the source marker and run `pnpm docs:generate`; do not hand-edit
generated documentation blocks.

- Redux Toolkit is the default state layer. The authoritative slice list is in
  `app/src/store/index.ts`.
- Persist user state through `userScopedStorage`, not ad hoc
  `localStorage`.
- Use `coreRpcClient` for core RPC. It `fetch()`es the loopback core
  directly and routes only non-loopback plain-`http://` runtimes (blocked as
  mixed content, #3865) through the `relay_http_rpc` Tauri command.
- Auth state comes from `CoreStateProvider` and
  `fetchCoreAppSnapshot()`.
- Routes are defined in `AppRoutes.tsx`. Check that file before adding links
  or redirects.
- Bundled agent prompts live under `crates/openhuman-core/src/agent/prompts/`, not in the
  frontend.

Analytics:

- Shared buttons use a stable, content-free `analyticsId`.
- Successful domain outcomes use `trackAnalyticsEvent` from
  `components/analytics`.
- Never send user text, entity IDs, filenames, credentials, or error messages.

UI rules:

- Use `useT()` for user-facing text and add real translations for every
  locale.
- Preserve interpolation placeholders across translations.
- Run `pnpm i18n:check`, `pnpm i18n:english:check`, and the i18n coverage
  test.
- Do not use dynamic imports in production `app/src`.
- Use `isTauri()` or catch `invoke` failures. Do not inspect
  `window.__TAURI__` directly.
- Canonical visual tokens live in `app/src/styles/tokens.css`.

## Tauri shell

Keep `crates/openhuman-app/` thin. The authoritative IPC list is the
`generate_handler!` call in `crates/openhuman-app/src/lib.rs`.

Do not add JavaScript injection to child webviews. New behavior belongs in
Rust-side IPC hooks. Audit new Tauri plugins for `js_init_script`.

The app uses Wry. Do not restore CEF or CDP scanner assumptions. The native
iMessage scanner remains separate because it reads `chat.db` directly.

## Rust domain structure

Business logic belongs under `crates/openhuman-core/src/<domain>/`. Do not add flat
`crates/openhuman-core/src/*.rs` domain files or business logic to
`crates/openhuman-core/src/core/`.

Preferred module shape:

| File | Purpose |
| --- | --- |
| `mod.rs` | Module declarations, re-exports, and controller aggregators |
| `types.rs` | Serde domain types |
| `store.rs` | Persistence |
| `ops.rs` | Business operations returning `RpcOutcome<T>` |
| `schemas.rs` | Controller schemas and thin handlers |
| `tools.rs` | Domain-owned agent tools |
| `bus.rs` | Event subscribers |
| `*_tests.rs` | Focused behavior tests |

Additional rules:

- Wire controllers through the registry in `crates/openhuman-core/src/core/all.rs`. Do not add
  namespace branches to `cli.rs` or `jsonrpc.rs`.
- RPC namespace strings are wire contracts and do not follow directory
  renames.
- Domain tools live with their domain and are re-exported through
  `crates/openhuman-core/src/tools/mod.rs`. Keep only cross-cutting tools in
  `tools/impl/`.
- Stable memory collection scope belongs in `metadata.path_scope`; item IDs
  are deduplication keys.
- Update `crates/openhuman-core/src/platform/about_app/` when user-visible capabilities
  change.
- `RpcOutcome<T>`, `StructuredRpcError`, `unwrap_rpc`, and the JSON-RPC HTTP
  client live in `crates/openhuman-rpc/`; the core re-exports the crate as
  `crate::rpc` (`pub use openhuman_rpc as rpc;` in
  `crates/openhuman-core/src/lib.rs`), and `openhuman-app` and `openhuman-tui`
  depend on it directly. Keep it free of business logic and core dependencies
  (its only deps are serde, serde_json, and optional log/reqwest/url behind
  the `http-client` feature).

## Tool, harness, and runtime boundaries

`tinyagents` owns tool-call dialects, parsing, catalog rendering, transcript
replay, session identity, and the agent loop. `tinytools` owns the shared
`Tool` trait and tool types. OpenHuman owns execution policy, approvals,
sandboxing, timeouts, and progress events.

- Use the `tinytools` copy vendored through `vendor/tinyagents/`; a second path
  creates incompatible Rust types.
- Keep conversions mechanical. Policy decisions belong in OpenHuman.
- **Put a change in the repo that owns it, not where it is easiest to land.**
  Tool-call parsing, grammars, the `Tool` trait and generic tool types go to
  `vendor/tinyagents/vendor/tinytools`; the agent loop, dialects, prompt
  cache layout, run policy, progress events and generic harness tools (the
  session todo list, goals, delegation graph) go to `vendor/tinyagents`
  (`tinyagents-harness` / `tinyagents-graph`); OpenHuman keeps only the host
  adapters (scope, dispatch, approvals, progress projection). Open the
  upstream PR in that repo first, then move the gitlink here. A host-side
  workaround for a harness or parser bug is a stopgap, not a fix: file or
  fix it upstream in the same PR.
- `openhuman_embed::Runtime` → `Agent` is the public library API: one runtime
  per process (features, services, backend URL, TinyHumans API key), then any
  number of independently configured agents on it (`AgentSpec`: provider,
  access, `action_dir`, MCP servers, skills, prompt, tool scope, sandbox).
  `Harness` is the one-agent shorthand over the same two types. Agent turns
  dispatch natively (`inference::local::ops::agent_chat_for`) under the
  agent's own `CoreContext` (`CoreContext::derive_with`); other facade calls
  go through `CoreRuntime::invoke`.
- Set `config_path` with `workspace_dir`, and set a turn origin with its access
  tier. `Access::full()` configures both access fields. Every agent on a
  runtime shares its `config_path` (credentials, keyring, API key).
- Copy skills into an agent's `personalities/<id>/skills/` (what
  `AgentSpec::skills_dir` does) because skill discovery rejects symlinked
  bundles. Library agents hide the operator's `~/.openhuman/skills` unless
  `include_user_skills(true)`.
- Library mode has no user login: the runtime's API key rides managed
  inference as `Authorization: Bearer` and backend REST as `x-api-key`
  (`security::credentials::api_key`, `session_support::BackendCredential`).
- The core never obtains, validates, exchanges or refreshes a credential.
  It takes one — a session JWT, an API key, or the offline local token —
  through `auth.set_credential` (`security::credentials::ops::credential`)
  and does only what it owns with it: user-dir activation, gated services,
  the scheduler gate, Sentry and prompt identity. Login-token exchange,
  `GET /auth/me` and the current-user cache belong to the host's session
  owner: `openhuman_tinyhumans::session` behind the Tauri shell's `auth_*`
  commands and the TUI, `openhuman_embed::Auth` for embedders, the CLI or
  `OPENHUMAN_BACKEND_API_KEY` / `OPENHUMAN_BACKEND_SESSION_TOKEN` for
  headless hosts. Do not add backend auth endpoints back to the core.

### Sessions

A conversation's durable identity is `tinyagents_session::transcript::SessionRef`,
derived from the thread id and the agent id. It maps to a transcript stem
deterministically and **without a timestamp**, so one conversation resolves to
one file in every process and on every launch. OpenHuman binds it in
`set_thread_id` and resumes with `ResumeMode::Session`; it does not mint stems,
seed history by hand, or pick a transcript by recency.

- **The transcript is what the model sees.** Trimming it is legitimate.
- **A compaction never erases.** It seals the current generation and opens the
  next (`begin_generation`), which records the sealed one as its parent. The
  sealed file stays on disk byte-for-byte, so the whole conversation is
  recoverable by walking `session_chain` even though the model reads only the
  head.
- **A resumed session's prompt is frozen.** It reuses its persisted system
  messages verbatim, which is what keeps the provider's prefix cache warm
  across a restart. The accepted consequence is that prompt edits, new skills
  and newly connected integrations do not reach an existing thread.
- **So is the tool list it was sent.** Every turn records its tool
  declarations in the transcript (a `{"kind":"tools"}` record, written only
  when they change); resume restores them, the prefix, and the committed-turn
  count. The host never shrinks a thread's tools because a cache went cold:
  `session_host/recorded_tools.rs` rebuilds recorded Composio actions as
  deferred executors, and the prelude fetches integrations on the first turn
  of every session instance, not only on a brand-new thread.
- **Pre-identity conversations are adopted once**, on first resume, from the
  timestamped stems they were written to (`adopt_legacy_session_transcripts`).
  No legacy file is modified.
- OpenHuman's session host keeps only the product surface over this: start a
  session, read it back, list its generations
  (`agent/session_host/session_api.rs`).

`CoreBuilder` controls background services with `ServiceSet`, runtime domains
with `DomainSet`, and tool visibility with `ToolGroups`. These controls only
narrow capabilities.

Cargo default features define the contributor build;
`scripts/ci/product-features.txt` defines the shipped product. The Tauri shell
disables default features, so product gates must be forwarded explicitly in
`crates/openhuman-app/Cargo.toml` and checked by
`scripts/ci/check-feature-forwarding.mjs`. Test both enabled and disabled
builds after changing a gate. Use `scripts/assert-shed.sh` or
`scripts/dep-sim.py` before claiming a dependency reduction.

## Loadable modules and bus contracts

Each loadable module has a small `*-bus` contract crate for interface names,
method constants, request and response types, and its contract version.

| Contract | Feature or role |
| --- | --- |
| `tinydocs-bus` | `documents` |
| `tinyvoice-bus` | `voice` |
| `tinyjuice-bus` | inference kernel |
| `tinyruntime-bus` | runtime clients |
| `tinywallet-bus` | `web3` |
| `tinymcp-bus` | `mcp` |
| `tinychannels-bus` | channel vocabulary |
| `tinyconnectors-bus` | OAuth connector (Composio) wire contract; called through `integrations/composio/module_client.rs` |

Rules:

- Never redeclare a contract type in OpenHuman.
- Call members through contract constants, not string literals.
- Contract crates stay synchronous and free of I/O and runtime dependencies.
- Shared wire behavior belongs in the contract. Runtime, config, and security
  policy stay in the host.
- Test the handwritten registry metadata against each contract's bus name and
  object path.
- Initialize recursive submodules before building: `git submodule update
  --init --recursive vendor/`.

Native modules are first-party `cdylib` files loaded into the core process.
They share its privileges and crash domain.

- Only the compiled registry may select artifacts.
- Pin release checksums from the published release. Do not compute replacement
  pins from a local build.
- Keep ABI, manifest, dependency, and digest admission checks.
- Do not unload or repeatedly retry a faulted module in the same process.
- Untrusted code belongs in a separate process.
- Do not enable the `modules` feature directly on the unconditional
  `tinybus` dependency. Forward it from OpenHuman's own feature.

Memory uses `tinymemory-api` as its contract. `memory::api` is a selective
re-export of the wire surface, not a place to copy or widen the whole crate.
Pass source scope and self-echo exclusions explicitly because task-local state
does not cross a module boundary. Confirm that a method exists in the pinned
module release before migrating a host call to it.

## Backend API

The core does not depend on `tinyhumans-sdk`. It reaches the hosted backend
only through the port `crates/openhuman-core/src/api/transport/`
(`BackendTransport`, `BackendRequest`, `BackendTransportError`); the SDK-backed
implementation is `crates/openhuman-tinyhumans` (`SdkBackendTransport`), which
sits above `openhuman-embed` and is installed once per process
(`openhuman_tinyhumans::install`, or `RuntimeBuilder` for library hosts, or
`CoreBuilder::backend_transport`). A core with no transport installed runs
agents, memory, tools and RPC without any TinyHumans connection and answers
backend-touching calls with `BackendApiError::BackendUnavailable` /
`BACKEND_UNAVAILABLE:`. Never add `tinyhumans-sdk` back to the core; the only
crate allowed to depend on it is `openhuman-tinyhumans` (`cargo tree -p
openhuman -i tinyhumans-sdk` must stay empty). Every host that boots a core
(`crates/openhuman-app/src/main.rs` and `lib.rs::run`,
`crates/openhuman-tui/src/runner.rs`, `crates/openhuman-cli/src/main.rs`)
calls `openhuman_tinyhumans::install` first; it also registers the hosted RPC
proxies (`billing`, `team`, `referral`, `announcements` —
`crates/openhuman-tinyhumans/src/hosted/`) into the core's controller
registry through `core::all::register_controller_extension`
(`DomainGroup::Hosted`). New backend-only proxy domains belong there, not in
the core.

Add missing backend routes to the vendored SDK (its unexposed-route registry
is the route policy the transport enforces) and name them from the core;
do not recreate route implementations in `crates/openhuman-core/src/api/`.

`crates/openhuman-core/src/api/` owns OpenHuman session-token lookup, base URL
selection, attribution headers and client profiles (`headers.rs`), and error
classification. Authenticated `BackendOAuthClient` requests go through
`authed_json`, whose private `finish_authed_json`
(`crates/openhuman-core/src/api/rest.rs`) classifies transient transport
failures and maps 401/404 responses to typed `BackendApiError` variants;
`IntegrationClient::map_transport_error`
(`crates/openhuman-core/src/integrations/client/errors.rs`) plays the same
role for integrations. Route new backend calls through those helpers instead
of matching `BackendTransportError` by hand.

Every TinyHumans backend request must carry a sanitized `x-sdk-name`:

- `BackendOAuthClient`
- `IntegrationClient`, except redirected file downloads
- the host session owner's `POST /auth/login-token/consume` and
  `GET /auth/me` (`openhuman_tinyhumans::session`, through `ClientHeaders`)
- the agent Langfuse ingestion request

Set `ProductIdentity` once during startup before building clients. Do not add
this header to third-party endpoints, MCP servers, BYOK inference endpoints, or
presigned storage redirects.

Search for `bearer_authorization_value` and `header(AUTHORIZATION` when
auditing hand-built backend requests.

## Event bus

`crates/openhuman-core/src/core/bus.rs` owns the process-wide `BUS` singleton. Use `BUS.publish` and
`BUS.subscribe` for domain events. Use `BUS.native()` for typed, in-process
request and response calls that carry values which cannot cross a serialized
transport.

Each subscribing domain owns a `bus.rs`. Subscriber names use
`<domain>::<purpose>`.

When adding an event:

1. Add it to `DomainEvent`.
2. Extend the `domain()` match.
3. Register its subscriber at startup.
4. Bump `EVENTS_VERSION` in `crates/openhuman-core/src/core/bus.rs`.

Native request and response types must be `Send + 'static` and do not need
serialization.

## Logging and code quality

- Prefer files under roughly 500 lines and split by responsibility.
- Add grep-friendly debug or trace logs for new flows, branches, external
  calls, retries, timeouts, state changes, and errors.
- Include useful correlation fields such as request IDs and method names.
- Never log credentials, tokens, full user content, or other sensitive data.
- Keep generated documentation synchronized with `pnpm docs:generate` and
  verify it with `pnpm docs:check`.
- Update code and documentation together when a contract changes.

## Git and platform notes

- Work happens on a branch, never directly on `main`.
- Push feature branches to the contributor fork and open PRs against
  `tinyhumansai/openhuman`.
- Use the issue and PR templates.
- Fix hook failures caused by your changes.
- `.husky/pre-push` runs `rust:clippy` only when the push carries Rust
  (`*.rs`, a manifest, `.cargo/`, `.gitmodules`, `crates/`, `vendor/`,
  `rust-toolchain.toml`); it runs
  anyway when the range cannot be resolved, or with
  `PRE_PUSH_FORCE_CLIPPY=1`. `scripts/__tests__/pre-push-hook.test.mjs`
  covers the hook.
- macOS deep links require a built app bundle.
- Windows registers `openhuman://` through `tauri-plugin-deep-link`.
- Standalone debugging uses `./target/debug/openhuman-core serve`. Public
  endpoints are `GET /health`, `GET /schema`, and `GET /events`, plus the
  debug-build-only `GET /dev/connect`.
