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
| `app/src-tauri/` | Thin desktop host |
| `src/core/` | Transport, dispatch, auth, and runtime composition |
| `src/openhuman/` | Business domains |
| `src/main.rs` | `openhuman-core` CLI |
| `tests/` | Rust integration and JSON-RPC tests |
| `gitbooks/` | Public product and contributor documentation |
| `docs/` | Internal maintainer documentation |
| `vendor/` | Recursive git submodules |

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
  `app/src-tauri/src/core_process.rs`. Frontend RPC uses the per-launch bearer
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
cargo build --manifest-path Cargo.toml --bin openhuman-core
cargo check --manifest-path app/src-tauri/Cargo.toml

# Apple Silicon workaround for llama.cpp
GGML_NATIVE=OFF cargo check --manifest-path Cargo.toml
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
`app/src-tauri/Cargo.toml`:

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
- E2E code must use `element-helpers.ts`, not raw platform element types.
- Tests must not call real backend or third-party services.
- Avoid time-based flakes and real network access in unit tests.

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
  `src/openhuman/config/schema/` and loaded through its config operations.

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
- Use `coreRpcClient` for core RPC. It delegates to the
  `relay_http_rpc` Tauri command.
- Auth state comes from `CoreStateProvider` and
  `fetchCoreAppSnapshot()`.
- Routes are defined in `AppRoutes.tsx`. Check that file before adding links
  or redirects.
- Bundled agent prompts live under `src/openhuman/agent/prompts/`, not in the
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

Keep `app/src-tauri/` thin. The authoritative IPC list is the
`generate_handler!` call in `app/src-tauri/src/lib.rs`.

Do not add JavaScript injection to child webviews. New behavior belongs in
Rust-side IPC hooks. Audit new Tauri plugins for `js_init_script`.

The app uses Wry. Do not restore CEF or CDP scanner assumptions. The native
iMessage scanner remains separate because it reads `chat.db` directly.

## Rust domain structure

Business logic belongs under `src/openhuman/<domain>/`. Do not add flat
`src/openhuman/*.rs` domain files or business logic to `src/core/`.

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

- Wire controllers through the registry in `src/core/all.rs`. Do not add
  namespace branches to `cli.rs` or `jsonrpc.rs`.
- RPC namespace strings are wire contracts and do not follow directory
  renames.
- Domain tools live with their domain and are re-exported through
  `src/openhuman/tools/mod.rs`. Keep only cross-cutting tools in
  `tools/impl/`.
- Stable memory collection scope belongs in `metadata.path_scope`; item IDs
  are deduplication keys.
- Update `src/openhuman/platform/about_app/` when user-visible capabilities
  change.

## Tool, harness, and runtime boundaries

`tinyagents` owns tool-call dialects, parsing, catalog rendering, transcript
replay, and the agent loop. `tinytools` owns the shared `Tool` trait and tool
types. OpenHuman owns execution policy, approvals, sandboxing, timeouts, and
progress events.

- Use the `tinytools` copy vendored through `vendor/tinyagents/`; a second path
  creates incompatible Rust types.
- Keep conversions mechanical. Policy decisions belong in OpenHuman.
- `openhuman_core::Harness` is the public prompt-to-reply API. Calls go through
  `CoreRuntime::invoke`, not directly to domain operations.
- Set `config_path` with `workspace_dir`, and set a turn origin with its access
  tier. `Access::full()` configures both access fields.
- Use one `Harness` per process. Copy skills into its workspace because skill
  discovery rejects symlinked bundles.

`CoreBuilder` controls background services with `ServiceSet`, runtime domains
with `DomainSet`, and tool visibility with `ToolGroups`. These controls only
narrow capabilities.

Cargo default features define the contributor build;
`scripts/ci/product-features.txt` defines the shipped product. The Tauri shell
disables default features, so product gates must be forwarded explicitly in
`app/src-tauri/Cargo.toml` and checked by
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

Backend calls use the vendored `tinyhumans-sdk`. Add missing backend routes to
that SDK rather than recreating them in `src/api/`.

`src/api/` owns OpenHuman session-token lookup, base URL selection, transport
configuration, and error classification. Every SDK error must pass through
`classify_sdk_error`.

Every TinyHumans backend request must carry a sanitized `x-sdk-name`:

- `BackendOAuthClient`
- `IntegrationClient`, except redirected file downloads
- `MedullaClient`, including its separate SSE handshake
- desktop `GET /auth/me`
- the agent Langfuse ingestion request

Set `ProductIdentity` once during startup before building clients. Do not add
this header to third-party endpoints, MCP servers, BYOK inference endpoints, or
presigned storage redirects.

Search for `bearer_authorization_value` and `header(AUTHORIZATION` when
auditing hand-built backend requests.

## Event bus

`src/core/bus.rs` owns the process-wide `BUS` singleton. Use `BUS.publish` and
`BUS.subscribe` for domain events. Use `BUS.native()` for typed, in-process
request and response calls that carry values which cannot cross a serialized
transport.

Each subscribing domain owns a `bus.rs`. Subscriber names use
`<domain>::<purpose>`.

When adding an event:

1. Add it to `DomainEvent`.
2. Extend the `domain()` match.
3. Register its subscriber at startup.
4. Bump `EVENTS_VERSION` in `src/core/bus.rs`.

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
