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

## Configuration and security

- Copy environment settings from `.env.example` and `app/.env.example`.
- Frontend environment access is centralized in `app/src/utils/config.ts`.
  Do not read `import.meta.env` elsewhere.
- Rust configuration is defined under
  `crates/openhuman-core/src/config/schema/` and loaded through its config operations.

The autonomy policy is security-sensitive:

- `action_dir` is the agent's permitted read and write root.
- `workspace_dir` stores internal state and is never an acting-tool target.
- Unknown commands classify as writes.
- System and credential paths are always forbidden.
- The approval gate is on by default. Interactive requests expire as denied
  after ten minutes.
- Sandboxed agents use the platform jail or Docker backend. Rust path checks
  still apply if the sandbox falls back.

Do not weaken `is_workspace_internal_path`, `is_always_forbidden`,
`classify_command`, or approval behavior to make a feature work.

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
replay, and the agent loop. `tinytools` owns the shared `Tool` trait and tool
types. OpenHuman owns execution policy, approvals, sandboxing, timeouts, and
progress events.

- Use the `tinytools` copy vendored through `vendor/tinyagents/`; a second path
  creates incompatible Rust types.
- Keep conversions mechanical. Policy decisions belong in OpenHuman.
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
- `MedullaClient`, including its separate SSE handshake
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
- macOS deep links require a built app bundle.
- Windows registers `openhuman://` through `tauri-plugin-deep-link`.
- Standalone debugging uses `./target/debug/openhuman-core serve`. Public
  endpoints are `GET /health`, `GET /schema`, and `GET /events`.
