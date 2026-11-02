# OpenHuman

OpenHuman is a React and Tauri v2 desktop assistant with an in-process Rust
core. The core also exposes JSON-RPC and a CLI.

Architecture: [overview](docs/gitbooks/en/developing/architecture.md),
[frontend](docs/gitbooks/en/developing/architecture/frontend.md),
[Tauri shell](docs/gitbooks/en/developing/architecture/tauri-shell.md), and
[agent harness](docs/gitbooks/en/developing/architecture/agent-harness.md).

## Repository map

| Path | Purpose |
| --- | --- |
| `app/src/` | Vite and React frontend |
| `crates/openhuman-app/` | Thin desktop host; excluded from the root workspace, build with `--manifest-path crates/openhuman-app/Cargo.toml`. Depends on `openhuman-rpc` only and boots its in-process core with `openhuman_rpc::host::desktop` |
| `crates/openhuman-core/` | Package `openhuman`: business domains under `src/<domain>/`, the controller contract, dispatch and auth under `src/core/` |
| `crates/openhuman-core/src/<domain>/` | Flat business-domain modules (agent, memory, tools, security, channels, ...) |
| `crates/openhuman-core/src/storage/` | The process's storage backend on the `tinystoragedrivers` ports: URL resolution (`OPENHUMAN_STORAGE_URL` / `[storage] url`), the install slot, agent scopes (`local` for the operator), and the base the domain stores build on. Features `storage-sqlite`, `storage-file`, `storage-mongodb`; see `docs/gitbooks/en/developing/architecture/storage.md` |
| `crates/openhuman-core/src/core/` | CLI, controller contract (`Outcome`, schemas) and in-process dispatch, controller registry, event bus, runtime composition; no business logic and no JSON-RPC server |
| `crates/openhuman-cli/` | The `openhuman-core` binary (`src/main.rs`, `openhuman_rpc::host::cli`), the ops bins (`src/bin/`: `openhuman-fleet`, `test-mcp-stub`), and every root `tests/*.rs` / `examples/*.rs` target. Normal dependency: `openhuman-rpc` only; the tests reach core, embed and tinyhumans through `[dev-dependencies]`. The benchmark bins live in the openhuman-benchmarks repository |
| `crates/openhuman-embed/` | Library facade over the core (depends on core only): `Runtime`/`RuntimeBuilder` with host presets, `embed::process` (tokio runtime, logging, dotenv, master key, Sentry options), and the curated facades hosts use (`config`, `artifacts`, `chat_surface`, `modules`, `identity`). Its doc-hidden `__host` list is for tinyhumans and rpc only |
| `crates/openhuman-rpc/` | Top of the library chain (depends on tinyhumans only). JSON-RPC 2.0 over the core: envelopes, HTTP client, and the server (router, Socket.IO, listener, `run_server*`); `host::{cli, desktop, tui}`, the shared host boot; re-exports `embed` and `tinyhumans` as the hosts' curated facade; plus `session_store` (`session-store` feature), the on-disk session store (`session_raw/`, `session_db/`, `tinyagents_store/`, turn states) behind TinyAgents' session store port, which the app, CLI and TUI install. Core and embed reach session state through the port (`agent::session_store`); a few legacy paths still fall back to workspace files when no store is installed. With a storage URL (`OPENHUMAN_STORAGE_URL` / `[storage] url`), `install_for_host` installs TinyAgents' `DriverSessionStores` over that backend instead (core `storage` domain) |
| `crates/openhuman-tinyhumans/` | The TinyHumans layer above embed (depends on embed only), and the only owner of anything hosted: the backend transport port and its SDK implementation, the authenticated client, routes and RPC proxies, and the hosted providers (integrations, voice, inference, channels, memory), all plugged into the core through its generic registries; a `RuntimeBuilder` that boots connected; and the host-side login/session owner (login-token exchange, `/auth/me`, current-user cache, credential handoff) used by app and TUI |
| `crates/openhuman-tui/` | Standalone terminal frontend; depends on `openhuman-rpc` only and boots with `openhuman_rpc::host::tui` |
| `tests/` | Rust integration and JSON-RPC tests |
| `docs/gitbooks/` | Public product and contributor documentation, synced with GitBook (`en/` is canonical, `zh/` the translation) |
| `docs/` | Everything else here is internal maintainer documentation |
| `vendor/` | Recursive git submodules; root `Cargo.toml` `[patch]` tables point into this tree |

Run commands from the repository root. The root package is a private pnpm
workspace.

The Rust crates form a strict chain; each one's normal dependencies name only
the layer directly below it:

```text
openhuman-core -> openhuman-embed -> openhuman-tinyhumans -> openhuman-rpc -> { app, cli, tui }
```

Hosts (app, CLI, TUI) depend on `openhuman-rpc` alone and reach the core
through its curated facade (`openhuman_rpc::host`, `openhuman_rpc::embed`,
`openhuman_rpc::tinyhumans`), never through `__host` / `core_host` or an
`openhuman_core::` path. `node scripts/ci/check-crate-chain.mjs` (part of
`pnpm rust:layout`) enforces both. Dev-dependencies are exempt, which is how
the root tests keep reaching into the core.

## Product boundaries

- The shipped Tauri product targets Windows, macOS, and Linux.
- The experimental iOS client is not part of the shipped desktop host. Its
  transport implementations live in `app/src/services/transport/`.
- The Rust core owns business rules, persistence, execution, RPC, and CLI
  behavior.
- The frontend and Tauri shell present or orchestrate core behavior. Do not
  duplicate core policy in TypeScript or shell code.
- The desktop core runs as a tokio task managed by
  `crates/openhuman-app/src/core_process.rs` (`openhuman_rpc::host::desktop`).
  Frontend RPC uses the per-launch bearer returned through the
  `core_rpc_token` command.
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
  dev-only `GET /dev/connect` (`crates/openhuman-rpc/src/server/dev_connect.rs`)
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

Scripted browser runs (Playwright, headless): `pnpm debug web` boots a
throwaway stack (the mock backend, a fresh `openhuman-core serve` on a scratch
workspace, Vite with `OPENHUMAN_VITE_NO_WATCH=1`) and signs in through the real
GitHub button, which the mock answers like the backend. `--script <file.mjs>`
runs a scenario against the signed-in page (default export receives `page`,
`mock.set(key, value)` for mock behaviors such as `llmStreamScript`, `rpc`,
`screenshot` and `log`); without it the stack stays up until Ctrl-C. Logs and
screenshots go to `target/debug-logs/web-<ts>/`. Example:
`scripts/debug/web-scripts/stop-mid-turn.mjs`. Set `OPENHUMAN_VITE_NO_WATCH=1`
on any Vite run that dies with `ENOSPC: System limit for number of file
watchers reached`. `--headed` needs the full Chromium build
(`pnpm --filter openhuman-app exec playwright install chromium`).

Long CI build or test commands must run through
`scripts/ci-cancel-aware.sh`. Do not export `CARGO_TARGET_DIR`; the repository
already configures shared build output where appropriate.

Keep matching profile settings synchronized between `Cargo.toml` and
`crates/openhuman-app/Cargo.toml`:

- Development dependencies use `debug = false`.
- Release builds use thin LTO, 16 codegen units (one unit pushed the desktop
  release matrix from ~46 to ~84 min, 6941b18c85), symbol stripping, and
  `debug = "line-tables-only"`.

## Testing and CI

CI Lite runs area-specific checks and changed-line coverage on PRs to `main`
or `release`. CI Full runs the complete suites for `release`. Changed-line
coverage must be at least 80 percent.

### Test ownership

OpenHuman tests composition, host policy, adapters, RPC wire contracts, and data compatibility. Module behavior is tested in the owning `vendor/tiny*` repository. A host test that calls a vendor module's internals (for example, `tinychannels::providers::*` or `tinyinference_local::service::*`) belongs upstream. When code moves upstream, move its tests in the same upstream PR, then delete them in the host PR. Keep one host smoke test per module: load it and make one call through the bus. That is enough to test the host integration.

A PR that only deletes tests is exempt from changed-line coverage because it has no changed lines for diff-cover to measure. CI Lite runs that changed-line check at an 80 percent threshold.

- Frontend unit tests are colocated as `*.test.ts` or `*.test.tsx` under
  `app/src/`. Use Vitest and test behavior rather than implementation.
- Rust unit tests are never inline. Put them in a sibling `<module>_tests.rs`
  (`mod_tests.rs` beside a `mod.rs`) declared at the bottom of the module with
  `#[cfg(test)]` and `#[path = "<module>_tests.rs"]` above `mod tests;`. The file
  starts with `use super::*;` and carries no `#[cfg(test)]` of its own. Never name
  one `test.rs`, `tests.rs` or `<module>_test.rs`, and never write an inline
  `#[cfg(test)] mod tests { ... }` (`pnpm rust:layout` fails on an inline module
  and on `test.rs`/`tests.rs`/`<module>_test.rs`). The same rule binds every
  `vendor/` submodule: `node scripts/externalize-inline-tests.mjs <repo-root>
  --write` converts one mechanically (add `--rename-legacy` for `test.rs` and
  `<module>_test.rs`), and a crate root directly in
  `src/bin/` keeps its tests in `src/bin/<stem>/` because Cargo builds any `.rs`
  placed straight in `src/bin/` as a binary.
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
  first (it runs `openhuman_tinyhumans::install`, a dev-dependency of
  `openhuman-cli`); the core has no hosted code of its own, and without it
  every backend call answers `BACKEND_UNAVAILABLE:`. Suites that spawn the
  `openhuman-core` binary get it from `main.rs` (`openhuman_rpc::host::cli`).

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
- `pnpm debug breakdown <req-NNN.json> --response <res-NNN.txt>`
  (`scripts/debug/prompt-breakdown.mjs`) prices one captured request: each
  system-prompt section, each tool schema (description vs parameters) and each
  message, in o200k tokens calibrated to the provider's `usage.prompt_tokens`,
  plus paragraphs the request pays for twice. Capture with
  `CAPTURE_ALL=1 CAPTURE_RESPONSES=1` so the response carries the usage.

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
- Always build UI from the shadcn primitives in `app/src/components/ui/`
  (`Button`, `Badge`, `Alert`, `Card`, `ModalShell`/`Dialog`, `Popover`,
  `Tooltip`, `Tabs`/`ChipTabs`, `TextField`, `NativeSelect`, `Toast`, ...).
  Do not hand-roll a surface a primitive already covers (a banner is an
  `Alert`, a dismiss control is a `Button`, a notification is `toast.add`).
  When a primitive is missing, add it to `components/ui/` from the shadcn
  registry (`components.json`, `base-nova` style), adapted to the app's
  tokens, `Button` and lucide icons, then use it.

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
| `ops.rs` | Business operations returning `Outcome<T>` |
| `schemas.rs` | Controller schemas and thin handlers |
| `tools.rs` | Domain-owned agent tools |
| `bus.rs` | Event subscribers |
| `*_tests.rs` | Focused behavior tests |

Additional rules:

- Wire controllers through the registry in `crates/openhuman-core/src/core/all.rs`. Do not add
  namespace branches to `cli.rs` or the JSON-RPC server.
- RPC namespace strings are wire contracts and do not follow directory
  renames.
- Domain tools live with their domain. Each domain builds its tool family
  through a registration function; `tools::ops` composes those families before
  applying capability filters. Preserve catalogue order when moving
  registrations. Import concrete tools from their owning domain instead of
  adding domain globs to `tools/mod.rs`. Keep only cross-cutting tools in
  `tools/impl/`.
- Memory scope comes from the namespace, never from metadata or model
  arguments: `memory::scope` resolves the acting identity to a layout root
  and memory agent id, and under layout v3 the engine is bound with
  `EngineSettings::scope_root` = `user:<id>` (`memory::scope::user_root`),
  below which TinyMemory places `ws:`/`source:`/`agent:` nodes and
  `app:<kind>` leaves (`vendor/tinymemory/docs/architecture/cortex-layout.md`).
  Item ids are deduplication keys.
- Update `crates/openhuman-core/src/platform/about_app/` when user-visible capabilities
  change.
- The controller contract lives in core: every domain operation returns
  `crate::core::Outcome<T>`; `crate::core::StructuredRpcError`, the params
  rules (`core::params`) and in-process dispatch (`core::invoke::invoke_method`)
  sit beside `ControllerSchema`. Core has no JSON-RPC server and does not
  depend on `openhuman-rpc`.
- `crates/openhuman-rpc/` sits above core and owns JSON-RPC 2.0: the envelopes
  (`RpcRequest`, `RpcSuccess`, `RpcFailure`, `request_body`,
  `decode_response`), the browser-origin allowlist, the HTTP client
  (`http-client` feature), and the whole server (`server` feature): the axum
  router and handlers, auth middleware, Socket.IO, `/dev/connect`, the
  listener bind (`openhuman_rpc::server::serve`) and the `run_server*` entry
  points. `openhuman_rpc::host::cli` gives the core this crate's server as
  the `run`/`serve` launcher.
  Domain-owned HTTP handlers the router mounts (`inference::http`, the
  dictation WebSocket) stay in their domains behind core's `http-server`
  feature. The `http_host` static-directory file server lives here too
  (`openhuman_rpc::http_host`); `host::cli`, `host::desktop` and
  `build_core_http_router()` register its `http_host.*` controllers as a core
  extension, so a host without this crate has no `http_host` surface.
- The hosts boot through `openhuman_rpc::host`: `host::cli(args)` is the
  `openhuman-core` binary (and the app's `core` / `mcp` subcommands);
  `host::desktop(DesktopOptions, shutdown, ready_tx)` is the desktop shell's
  embedded server (in-memory bearer, preferred port with stale-listener
  takeover, ready signal); `host::tui()` builds the TUI's runtime. Each
  connects the TinyHumans backend itself. One embed runtime exists per
  process, so a host that restarts its server must let the old task (and the
  runtime it owns) drop before it spawns the next.

## Tool, harness, and runtime boundaries

Embed public API documentation: [Embedding](docs/gitbooks/en/developing/embed/README.md),
[concepts](docs/gitbooks/en/developing/embed/concepts/README.md), and the source-generated
[builder setters](docs/gitbooks/en/developing/embed/builder-setters.md). Snippets come from
compiled examples; run `pnpm docs:generate` and `pnpm docs:check` after changing them.


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
  dispatch natively (`inference::host_runtime::ops::agent_chat_for`) under the
  agent's own `CoreContext` (`CoreContext::derive_with`); other facade calls
  go through `CoreRuntime::invoke`.
- Set `config_path` with `workspace_dir`, and set a turn origin with its access
  tier. `Access::full()` configures both access fields. Every agent on a
  runtime shares its `config_path` (credentials, keyring, API key).
- Copy skills into an agent's `agents/<id>/skills/` (what
  `AgentSpec::skills_dir` does) because skill discovery rejects symlinked
  bundles. Library agents hide the operator's `~/.openhuman/skills` unless
  `include_user_skills(true)`.
- Library mode has no user login: the runtime's API key rides managed
  inference as `Authorization: Bearer` and backend REST as `x-api-key`
  (`credentials::api_key`, `session_state::BackendCredential`).
  Every backend caller resolves its credential through
  `resolve_backend_credential` (or `backend_bearer_secret` for bearer-only
  seams), never `get_session_token`, so the key covers integrations, voice,
  embeddings, memory-host and socket calls too; only `/auth/*` session flows
  need a signed-in user.
  Every backend caller resolves its credential through
  `resolve_backend_credential` (or `backend_bearer_secret` for bearer-only
  seams), never `get_session_token`, so the key covers integrations, voice,
  embeddings, memory-host and socket calls too; only `/auth/*` session flows
  need a signed-in user.
- The core never obtains, validates, exchanges or refreshes a credential.
  It takes one — a session JWT, an API key, or the offline local token —
  through `auth.set_credential` (`credentials::ops::credential`)
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
  and newly connected integrations do not reach an existing thread, unless
  the agent's definition sets `refresh_prompt_on_resume`: then a resumed
  thread is sent the prompt rendered now, and a changed prompt seals the
  current generation and continues the conversation in the next
  (`agent/session_host/runtime_session/prompt_refresh.rs`).
- **So is the tool list it was sent.** Every turn records its tool
  declarations in the transcript (a `{"kind":"tools"}` record, written only
  when they change); resume restores them, the prefix, and the committed-turn
  count. The host never shrinks a thread's tools because a cache went cold:
  `session_host/recorded_tools.rs` rebuilds recorded Composio actions as
  deferred executors, and the prelude fetches integrations on the first turn
  of every session instance, not only on a brand-new thread. Explicit embed
  `Turn::tools` overrides and host-only belts are authoritative instead: they
  do not restore revoked tools from an earlier transcript.
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
disables default features, so product gates must be forwarded explicitly on
its `openhuman-rpc` dependency in `crates/openhuman-app/Cargo.toml` and
checked by `scripts/ci/check-feature-forwarding.mjs`. The same gate checks the
library chain: a core gate must be forwarded by `openhuman-embed`, then
`openhuman-tinyhumans`, then `openhuman-rpc`, then the `openhuman-cli` and
`openhuman-tui` hosts (each to `openhuman-rpc/<gate>`), or be listed in

## Storage engine ownership

tinystoragedrivers (`vendor/tinystoragedrivers`) is the only crate family that may
depend on a storage engine. Engines (rusqlite, mongodb, aws-sdk-s3, lapin,
object_store, sqlx, redb, sled, rocksdb, fs2, and their transitive deps) are
reachable only from the following composition roots, which are the only
packages listed in the ALLOWLIST in `scripts/ci/check-storage-owner.mjs`:

  openhuman          core storage domain (installs and owns the backend)
  openhuman-app      keyring secrets via rusqlite
  openhuman-cli      keyring secrets via rusqlite
  openhuman-tui      keyring secrets via rusqlite
  openhuman-rpc      keyring secrets via rusqlite
  tinychannels       iMessage chat.db reader (read-only SQLite)
  tinybox-cli        bin target, bundles engine
  tinyjuice          bin target, bundles engine

Every other first-party crate must reach tinystoragedrivers only through
`tinystoragedrivers-types`, `tinystoragedrivers-api`, `tinystoragedrivers-bus`
or `tinystoragedrivers-busproxy`. The gate `node scripts/ci/check-storage-owner.mjs`
walks the normal+build dependency closure of `--all-features` from every
non-root first-party package and fails if it reaches an engine crate.
Run in `pnpm rust:layout`.

`CHAIN_GATES_NOT_FORWARDED` / `CHAIN_LOCAL_GATES` with a reason. Test both enabled and disabled
builds after changing a gate. Use `scripts/assert-shed.sh` or
`scripts/dep-sim.py` before claiming a dependency reduction.

## Loadable modules and bus contracts

Hosts must interface with loadable components through their minimal `*-bus` contracts. Never import, re-export, link, or call their implementation libraries directly, including through wrapper crates or indirect dependencies. Execute component behavior through the compiled TinyBus module. If the required types or operations are missing, extend the contract and implementation in the owning repository and raise a PR against its canonical upstream first; then consume the released module and update OpenHuman’s gitlink and adapter. Do not introduce a host-side implementation workaround.

### Submodule ownership

OpenHuman is the host and orchestrator for these components. It composes them,
loads modules, adapts their contracts to product RPC/tools, and applies
OpenHuman-specific configuration, security policy, approvals, and lifecycle
rules. It is not the implementation home for behavior that belongs to a
vendored project.

Before changing code, identify the owning repository below. Implement a
module/library capability, bug fix, or contract change in that submodule,
raise its PR against that repository's canonical upstream, and update the
OpenHuman gitlink only after the upstream change is available. OpenHuman may
contain the host adapter and integration tests that prove the composition, but
do not copy the module implementation into OpenHuman or add a host-side
workaround for a defect owned by a submodule. For a change that spans a module
and its host adapter, make both changes in their respective repositories and
raise the module PR first. Keep PRs and gitlinks independently reviewable.

Direct rendered submodules under `vendor/`:

| Submodule | Owns |
| --- | --- |
| `tiny-artifacts` | Versioned artifacts as a plain library (no bus contract): the record and version model, the `Artifacts` service and its rules (content fits the kind, file paths stay inside the host's `FileRoots` and pass its guard, revisions append, kept version files, publishing), the agent artifact tools, and the file-system store. OpenHuman owns the ports: the document-backed store, the card UI bridge, the protected-path guard and the cloud-storage publisher (`artifacts/`). |
| `tinyagents` | Provider-neutral agent harness and durable typed state graph: model/tool loop, tool-call dialects and parsing, middleware, retries, caching, sessions/transcripts, and graph execution. |
| `tinybox` | Isolated execution environments for code the host does not trust; box lifecycle and isolation backends. |
| `tinybus` | TinyBus runtime and module contracts: discovery/loading, ABI and manifest admission, transport, proxies, lifecycle, and module bus behavior. |
| `tinychannels` | Portable channel/message contracts, configuration/schema, routing metadata, and channel backend abstractions. OpenHuman owns its concrete product/backend adapters. |
| `tinyconnectors` | OAuth connector module behavior: account linking, available actions, action execution, and connector webhooks. |
| `tinycomputer` | Computer use as one TinyBus module: native desktop accessibility observation and interaction, browser sessions and control (the `Browser*` members), and tasks (`StartTask`/`AwaitTask`/`ContinueTask`) driven by a selectable decision model (Jev, OpenJev, Sage) with planner and rescue models. |
| `tinydocs` | Document extraction and synthesis, including PDF reading and DOCX/PPTX generation. |
| `tinyflows` | Host-agnostic workflow graph definition, validation, compilation, and execution engine. |
| `tinyhosts` | Hosting provider APIs and deployment/database/domain/analytics operations, as library and TinyBus module. |
| `tinyhumans-sdk` | Rust client types and transport operations for the public TinyHumans backend API. OpenHuman owns its transport integration and product auth/session policy. |
| `tinyjuice` | Agent tool-output compression and recovery of omitted content. |
| `tinymcp` | The TinyMCP module implementation and its bus contract. Put MCP module behavior and contract changes here; OpenHuman owns configuration, lifecycle, and host integration. |
| `tinymemory` | Engine-neutral memory contracts, operations, and providers. Its nested TinyCortex submodule owns the TinyCortex memory engine. |
| `tinystoragedrivers` | Storage ports, drivers (sqlite/file/mongodb/s3), secrets, storage bus contract; every module's tsd dependency is patched onto this checkout. |
| `tinysearch` | Web-search module, provider dispatch, tool declarations, and execution behind its TinyBus contract. |
| `tinysecurity` | Security decisions exposed by `tinysecurity-bus`: command and path policy, approval lifecycle and trust, action budgets, redaction and scans, privacy and egress, sandbox planning, audit, keyring and secret crypto, and device pairing. OpenHuman owns configuration translation, host callbacks, the approval surface, and execution orchestration. |
| `tinyskills` | Host-independent skill/workflow bundle parsing, discovery, scope resolution, resource inventory, and safe reads. OpenHuman owns trust and execution policy. |
| `tinyvoice` | Host-agnostic voice primitives such as audio framing, VAD, wake-word gating, routing, and STT hallucination detection. |
| `tinywallet` | Multi-chain wallet: `tinywallet-crypto` (address, asset, chain, `rpc::Transport`, tx codec), `tinywallet-x402` (x402 wire, payment, spending ledger, `x402_request` tool), `tinywallet-web3` (wallet engine, per-chain build/sign/broadcast flows, swap/bridge/dapp quotes, agent tools) behind host seams (`WalletSigner`, `PaymentSigner`, `WalletAccounts`, `RpcEndpoints`, `QuoteScope`, `Web3Backend`, `ProxyPolicy`), and the loadable `tinywallet-module` that derives keys and signs. OpenHuman keeps keyring, consent, credentials, config, controllers and the seam impls under `web3/`. |

Some rendered submodules are shared dependencies nested inside those projects,
not separate OpenHuman feature implementations. Make changes to them in their
own canonical repositories as well:

| Nested submodule | Owns |
| --- | --- |
| `tinyagents/vendor/tinytools` | Shared `Tool` trait and generic tool types. This is the single `tinytools` copy used by OpenHuman. |
| `tinyagents/vendor/tinyinference` and `tinymemory/vendor/tinyinference` | Inference/provider, embedding, local model, and voice inference libraries. OpenHuman patches the TinyAgents copy in its Cargo workspace; do not create a competing copy. |
| `tinymemory/vendor/tinycortex` | TinyCortex engine implementation for the TinyMemory contracts. |
| `*/vendor/tinybus` | Shared TinyBus contract/runtime dependency; change the owning TinyBus project, not a vendored duplicate. |
| `tinycomputer/vendor/agent-browser` | Browser-control library used by TinyComputer. |
| `tinycomputer/vendor/agent-desktop` | Cross-platform desktop accessibility and interaction library used by TinyComputer. |
| `*/vendor/tinyjevclient` | Shared TinyJEV client used by the browser and desktop modules. |
| `tinyagents/wiki`, `tinychannels/wiki`, `tinyjuice/wiki` | Project documentation content, not runtime implementation. |

When ownership is unclear, inspect the submodule's README, crate boundaries,
and bus contract before editing. A behavior change belongs with the code that
defines that behavior; OpenHuman changes should be limited to the host-side
composition and policy described above.

Each loadable module has a small `*-bus` contract crate for interface names,
method constants, request and response types, and its contract version.

| Contract | Feature or role |
| --- | --- |
| `tinydocs-bus` | `documents` |
| `tinyvoice-bus` | `voice` |
| `tinyjuice-bus` | inference kernel |
| `tinywallet-bus` | `web3` (contract; the chain primitives are in `tinywallet-crypto`) |
| `tinymcp-bus` | `mcp` |
| `tinychannels-bus` | channel vocabulary |
| `tinyconnectors-bus` | OAuth connector (Composio) wire contract; called through `integrations/composio/module_client.rs` |
| `tinysecurity-bus` | Security policy, approval, audit, redaction, egress, sandbox-plan, keyring, secret-cipher and device-pairing contracts; called through `modules/security/` |

Rules:

- Never redeclare a contract type in OpenHuman.
- Call members through contract constants, not string literals.
- Contract crates stay synchronous and free of I/O and runtime dependencies.
- Shared wire vocabulary belongs in the contract. Component algorithms and
  security decisions execute in the compiled module. Runtime configuration,
  host callbacks, user interaction, and external-effect execution stay in the
  host.
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

Module sources (`OPENHUMAN_MODULE_SOURCE`, `[modules] source`; details in
`crates/openhuman-core/src/modules/README.md`, "Module source"):

- `release` is the registry-pinned, digest-checked GitHub artifact. CI and
  shipped builds use `release` only; never document or configure `linked` or
  `source` for CI or production.
- `linked` (in-process factory) and `source` (a locally built artifact from the
  gitlink) are local development and local tests only.
- Iterate with `scripts/modules/build.mjs`, check the pin with
  `scripts/modules/pin-status.mjs` before pushing, and release and re-pin with
  `scripts/modules/release-and-repin.mjs`.

`[components]` (`config.toml`, `OPENHUMAN_COMPONENT_<NAME>=0|1`) holds one bool
per tiny dependency, all default true. It resolves to a `ComponentSet` that only
narrows the host's `DomainSet`, `ServiceSet` and `ToolGroups`, and takes effect
on the next core start. It adds no Cargo gate. A disabled component's module is
never downloaded, mapped or registered (`ModuleState::Disabled`,
`LoadError::Disabled`), its tools are not registered, its RPC namespace answers
`COMPONENT_DISABLED:<name>`, and `about_app` marks its capabilities `disabled`.
See `docs/gitbooks/en/developing/components.md`.

Memory uses `tinymemory-api` as its contract. `memory::api` is a selective
re-export of the wire surface, not a place to copy or widen the whole crate.
Pass source scope and self-echo exclusions explicitly because task-local state
does not cross a module boundary. Confirm that a method exists in the pinned
module release before migrating a host call to it.

## Backend API

The core has no backend module, no hosted port, no hosted route and no
TinyHumans hostname (`crates/openhuman-core/src/backend/` is gone, and
`node scripts/ci/check-core-hosted-free.mjs`, part of `pnpm rust:layout`, fails
on `crate::backend`, `tinyhumans.ai` and the hosted route literals
`agent-integrations`, `/voice-agent`, `/openai/v1`, `/telemetry/langfuse`,
`/memory/free-period` and `/teams/me` in core source outside comments and
`_tests.rs`). The core does not depend on `tinyhumans-sdk`; the only crate that
may is `openhuman-tinyhumans` (`cargo tree -p openhuman -i tinyhumans-sdk` must
stay empty).

`crates/openhuman-tinyhumans` owns everything hosted: the transport port
(`BackendTransport`, `BackendRequest`, `BackendTransportError`,
`TransportProfile`, `BaseUrlPurpose` in `src/transport/port.rs`) and its
SDK-backed implementation (`SdkBackendTransport`), the authenticated client
(`src/client/`: `BackendClient`, `HostedApiError`, `flatten_authed_error`), the
URL, header and product-identity policy (`src/backend/`: `url.rs`, `headers.rs`,
`product.rs`), the hosted routes and RPC proxies (`src/hosted/`), and the hosted
providers (`src/integrations/`, `src/voice/`, `src/inference/`,
`src/channels/`, `src/memory/`). It builds one process-wide transport
(`openhuman_tinyhumans::client::shared_transport`); the core holds no transport
slot (`CoreBuilder::backend_transport` and the `CoreContext` accessor are
removed). What a backend response *means* is decided in the transport
(`tinyhumans_sdk::classify`, applied by `map_sdk_error`); the recovery lives in
`BackendClient` and `IntegrationClient::map_transport_error`
(`crates/openhuman-tinyhumans/src/integrations/client/errors.rs`). Add missing
backend routes to the vendored SDK (its unexposed-route registry is the route
policy the transport enforces), not to the core.

Tinyhumans plugs into the core only through generic registries, keyed by a
slug or id, that know nothing about TinyHumans and that any other host could
fill. `openhuman_tinyhumans::install` (or `RuntimeBuilder::connect`) registers
them through `register_all` (`src/register.rs`), reaching the core via
`openhuman_embed::__host`:

| Hook | Core module path |
| --- | --- |
| Agent tool sources (media, file storage, integrations, google places, stock prices, web3) | `tools::sources` (`register_tool_source`) |
| STT, TTS and viseme providers by slug | `voice::factory::registry` |
| Live voice sessions by slug | `voice::live::registry` |
| Chat models, embeddings and model catalogs by provider id | `inference::registry` |
| Channel callback (`ChannelBackendRequest` to `ChannelBackendResult`) | `channels::callback_slot` |
| Config enricher filling `Config.resolved_endpoints` | `config::register_enricher` (`config/enrichers.rs`) |
| Memory engines by id and the free-background-work predicate | `memory::engine_registry` |
| Composio route builder by mode | `integrations::composio::client::register_composio_route_builder` |
| Connector proxy target by mode | `modules::connectors::register_proxy_target` |
| Wallet backend handler | `modules::wallet_backend` (`register_wallet_backend`) |
| Hosted RPC proxies (`billing`, `team`, `referral`, `announcements`, `webhooks`, `channel_link`, `oauth`) | controller extension via `core::all::register_controller_extension` (`DomainGroup::Hosted`) |

Registrations bind per process, or per `CoreContext` through
`CoreBuilder::extensions` (the `*_in` variants). A core with nothing registered
runs agents, memory, tools and RPC without any TinyHumans connection; hosted
surfaces are missing and backend-touching calls answer `BACKEND_UNAVAILABLE:`
(the generic sentinel prefixes stay in `core::observability`). The core reads
resolved URLs, the socket URL, the tracing ingestion URL and headers, the
attribution headers and the product identity from `Config.resolved_endpoints`,
which the tinyhumans enricher fills on every config load; it never asks a
transport. `Config.api_url` is honoured as an override exactly as before.

Every host that boots a core (`crates/openhuman-app/src/core_process.rs` and
`lib.rs::run_core_from_args`, `crates/openhuman-tui/src/runner.rs`,
`crates/openhuman-cli/src/main.rs`) does it through an `openhuman_rpc::host`
entry, which connects the TinyHumans layer
(`openhuman_tinyhumans::RuntimeBuilder::connect`); library hosts call
`openhuman_tinyhumans::install` or the `RuntimeBuilder` directly. Tests that
boot the core in-process and reach the mock backend call
`tinyhumans_boot::boot()`. New backend-only proxy domains belong in
`crates/openhuman-tinyhumans/src/hosted/`, not in the core.

**Hosted memory.** The `tinyhumans` memory engine is registered through
`memory::engine_registry` by `crates/openhuman-tinyhumans/src/memory/`. It
builds TinyMemory's CortexDB engine with `EngineSettings { endpoint, headers,
scope_root, .. }` and `EngineCredential::Dynamic(HostBearer)`, and TinyMemory's
own `reqwest` client (`tinymemory-integrations/src/cortex/transport`) calls the
backend's memory routes directly: none of those requests pass through
`SdkBackendTransport`, so transport-level policy (the route registry,
`map_sdk_error`) does not apply to them. The credential comes from
`resolve_backend_credential` on every request. The `cortexdb` engine likewise
calls CortexDB directly with the user's key.

Every TinyHumans backend request must carry a sanitized `x-sdk-name`, stamped by
the tinyhumans attribution headers (`src/backend/headers.rs`):

- `BackendClient`
- `IntegrationClient`, except redirected file downloads
- the host session owner's login-token consume and current-user requests
  (`openhuman_tinyhumans::session`, through `ClientHeaders`)
- the agent's Langfuse and OTLP ingestion push
- the managed model catalog listing

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

<!-- gitbook-agent-instructions:start -->

## GitBook Documentation Editing

This repository contains documentation synced with GitBook via Git Sync.

Before editing GitBook-synced Markdown, YAML, or asset files, make sure the GitBook skill is available and up to date in your local agent environment. Prefer installing or updating it with:

```bash
npx skills add gitbookio/gitbook-skills
```

This command may add or update local agent skill files. Use them only as local agent instructions; do not commit those installed skill files or any tool-generated agent configuration unless the user explicitly asks for it.

If `npx` is unavailable, load the skill from:

https://gitbook.com/docs/skill.md

When making changes, preserve GitBook sync metadata such as frontmatter, `SUMMARY.md`, `gitbook-docs.yaml`, `.gitbook/`, and asset links unless the requested edit explicitly requires changing them.

<!-- gitbook-agent-instructions:end -->
