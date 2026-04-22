# OpenHuman

**AI-powered assistant for communities — React + Tauri v2 desktop app with a Rust core (JSON-RPC / CLI) and sandboxed QuickJS skills.**

This file orients contributors and coding agents. Authoritative narrative architecture: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). Frontend layout: [`docs/src/README.md`](docs/src/README.md). Tauri shell: [`docs/src-tauri/README.md`](docs/src-tauri/README.md).

## Detailed guides (read on demand)

- **Testing** (unit + E2E + mock backend + Rust tests): [`docs/TESTING.md`](docs/TESTING.md)
- **Feature workflow + debug logging + controller migration + platform notes**: [`docs/feature-workflow.md`](docs/feature-workflow.md)
- **Event bus** (`DomainEvent` pub/sub and `NativeRegistry` typed in-process dispatch): [`docs/event-bus.md`](docs/event-bus.md)
- **Design tokens**: [`docs/DESIGN_GUIDELINES.md`](docs/DESIGN_GUIDELINES.md)

---

## Repository layout

| Path                    | Role                                                                                                                                                                                                        |
| ----------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **`app/`**              | Yarn workspace **`openhuman-app`**: Vite + React (`app/src/`), Tauri desktop host (`app/src-tauri/`), Vitest tests                                                                                          |
| **Repo root `src/`**    | Rust library **`openhuman_core`** and **`openhuman`** CLI binary entrypoint (`src/main.rs`) — `core_server`, `openhuman::*` domains, skills runtime (QuickJS / `rquickjs`), MCP routing in the core process |
| **Skills registry**     | **[`tinyhumansai/openhuman-skills`](https://github.com/tinyhumansai/openhuman-skills)** on GitHub — canonical skill packages and TS build; not vendored in this tree.                                       |
| **`Cargo.toml`** (root) | Core crate; `cargo build --bin openhuman` produces the sidecar the UI stages via `app`'s `core:stage`                                                                                                       |
| **`docs/`**             | Architecture and module guides (numbered pages under `docs/src/`, `docs/src-tauri/`)                                                                                                                        |

Commands in documentation assume the **repo root** unless noted: `yarn dev` runs the `app` workspace.

**Skills registry:** Skill sources and the bundler live in **[github.com/tinyhumansai/openhuman-skills](https://github.com/tinyhumansai/openhuman-skills)**. Clone that repository to author or change skills (`yarn install`, `yarn build`). The desktop app's skills catalog defaults to that GitHub slug; override with `VITE_SKILLS_GITHUB_REPO` (see [`app/src/utils/config.ts`](app/src/utils/config.ts)).

---

## Runtime scope

- **Shipped product**: desktop — Windows, macOS, Linux (see [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) "Platform reach").
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

**Tests**: Vitest in `app/` (`yarn test`, `yarn test:coverage`). Rust tests via `cargo test` at repo root as wired in `app/package.json`. See [`docs/TESTING.md`](docs/TESTING.md) for the full guide.

**Quality**: ESLint + Prettier + Husky in the `app` workspace.

---

## Configuration

Environment variables are documented in two `.env.example` files:

- **[`.env.example`](.env.example)** (repo root) — Rust core, Tauri shell, backend URL, logging, proxy, storage, web search, local AI binary overrides. Loaded via `source scripts/load-dotenv.sh`.
- **[`app/.env.example`](app/.env.example)** — Frontend `VITE_*` vars (core RPC URL, backend URL, Sentry DSN, skills repo, dev helpers). Copy to `app/.env.local` for local overrides.

**Frontend config** is centralized in [`app/src/utils/config.ts`](app/src/utils/config.ts). All `VITE_*` env vars should be read there and re-exported — do not read `import.meta.env` directly in other files.

**Rust config** uses a TOML-based `Config` struct (`src/openhuman/config/schema/types.rs`) with env var overrides applied in `src/openhuman/config/schema/load.rs`. Env vars override config file values at runtime (e.g. `OPENHUMAN_API_URL` overrides `config.api_url`).

---

## Frontend (`app/src/`) — quick map

- **Provider chain** (`App.tsx`, order matters for auth and realtime): `Redux Provider` → `PersistGate` → **`UserProvider`** → **`SocketProvider`** → **`AIProvider`** → **`SkillProvider`** → **`HashRouter`** → `AppRoutes`. There is **no** `TelegramProvider`; MTProto is not an active provider here.
- **State** (`store/`): Redux Toolkit slices — **auth**, **user**, **socket**, **ai**, **skills**, **team**. Prefer Redux (and persist where configured) over ad hoc `localStorage`.
- **Services** (`services/`): singleton-style — **`apiClient`**, **`socketService`**, **`coreRpcClient`** (HTTP bridge to the core process), domain **`api/*`** clients. No `mtprotoService`.
- **MCP** (`lib/mcp/`): transport/validation/types for JSON-RPC-style messaging over Socket.io — **not** a Telegram tool pack. Agent tooling is driven by the **skills** system and backend.
- **Routes** (`AppRoutes.tsx`): hash routes `/`, `/onboarding`, `/mnemonic`, `/home`, `/intelligence`, `/skills`, `/conversations`, `/invites`, `/agents`, `/settings/*`, plus `DefaultRedirect`. **No** dedicated `/login` route.
- **AI configuration**: bundled prompts live under **`src/openhuman/agent/prompts/`** at the **repository root** (also bundled via `app/src-tauri/tauri.conf.json` `resources`). Loaders under `app/src/lib/ai/` use `?raw` imports, optional remote fetch, and in Tauri **`ai_get_config` / `ai_refresh_config`** for packaged content.

---

## Tauri shell (`app/src-tauri/`)

Thin desktop host: window management, daemon health bridging, **core process lifecycle** (`core_process`, `CoreProcessHandle`), and **JSON-RPC relay** to the **`openhuman`** sidecar (`core_rpc_relay`, `core_rpc`).

Registered IPC commands (see [`docs/src-tauri/02-commands.md`](docs/src-tauri/02-commands.md)) include **`greet`**, **`write_ai_config_file`**, **`ai_get_config`**, **`ai_refresh_config`**, **`core_rpc_relay`**, **window** commands, and **OpenHuman service / daemon host** helpers (`openhuman_*`).

Deep link plugin is registered where supported; behavior is platform-specific (see [`docs/feature-workflow.md`](docs/feature-workflow.md) platform notes).

---

## Rust core (repo root `src/`) — key rules

- **`openhuman/`** — Domain logic (skills, memory, channels, config, …). RPC controllers live in **`rpc.rs`** files per domain; use **`RpcOutcome<T>`** pattern per [`AGENTS.md`](AGENTS.md) / internal rules.
- **`src/openhuman/` module layout**: **New** functionality must live in a **dedicated subdirectory** (its own folder/module, e.g. `openhuman/my_domain/mod.rs` plus related files, or a new subfolder under an existing domain). Do **not** add new standalone `*.rs` files directly at `src/openhuman/` root.
- **Controller schema contract**: Shared types live in **`src/core/mod.rs`** (`ControllerSchema`, `FieldSchema`, `TypeSchema`); domain metadata in a dedicated module inside the domain folder (example: **`src/openhuman/cron/schemas.rs`**), exported from the domain `mod.rs`.
- **Controller-only exposure rule**: Expose domain functionality to **CLI and JSON-RPC through the controller registry** (`schemas.rs` + registered handlers). Do **not** add domain-specific branches or one-off transport logic in `src/core/cli.rs` or `src/core/jsonrpc.rs`.
- **Light `mod.rs` rule**: Keep domain `mod.rs` files light and export-focused. Put operational code in sibling files (example: `ops.rs`, `store.rs`, `schedule.rs`, `types.rs`), then re-export the public API from `mod.rs`.
- **`core_server/`** — Transport only: Axum/HTTP, JSON-RPC envelope, CLI parsing, **dispatch** (`core_server::dispatch`) — **no** heavy business logic here.
- **Layering**: Implementation in `openhuman::<domain>/`, controllers in `openhuman::<domain>/rpc.rs`, routes in `core_server/`.

Skills runtime uses **QuickJS** (`rquickjs`) in **`src/openhuman/skills/`** (e.g. `qjs_skill_instance.rs`, `qjs_engine.rs`), not V8/deno_core in this repository.

Event bus and controller migration details: [`docs/event-bus.md`](docs/event-bus.md), [`docs/feature-workflow.md`](docs/feature-workflow.md).

---

## Desktop shell (Tauri) vs application code

In the parent **OpenHuman** desktop app, **Tauri / Rust is a delivery vehicle**: windowing, process lifecycle, IPC to the core sidecar, and other host concerns. **Keep as much UI behavior and product logic as practical in TypeScript/React** (`app/`). Avoid growing Rust in the shell for flows that belong in the web layer unless there is a hard platform or security reason.

---

## Git workflow

- **GitHub issues + PRs on upstream** — File on **[tinyhumansai/openhuman](https://github.com/tinyhumansai/openhuman/)** ([Issues](https://github.com/tinyhumansai/openhuman/issues), [PRs](https://github.com/tinyhumansai/openhuman/pulls)), not only a fork's tracker, unless the workflow explicitly says otherwise.
- **Templates** — Use [`.github/ISSUE_TEMPLATE/feature.md`](.github/ISSUE_TEMPLATE/feature.md) / [`bug.md`](.github/ISSUE_TEMPLATE/bug.md) for issues and [`.github/PULL_REQUEST_TEMPLATE.md`](.github/PULL_REQUEST_TEMPLATE.md) for PRs. AI-generated text should follow them verbatim.
- **Public repo**; push to your working branch; PRs target **`main`**.

---

## Coding philosophy

- **Unix-style modules**: individual modules with a single, sharp responsibility. Compose via small, well-named units.
- **Tests before the next layer**: ship enough unit tests for new behavior before stacking more on top. Treat untested code as incomplete.
- **Documentation with code**: new or changed behavior ships with matching docs. Update `AGENTS.md`, architecture docs, or feature docs when repository rules or user-visible behavior change.

---

## Key patterns

- **`src/openhuman/`**: new features go in a folder/module, not new root-level `src/openhuman/*.rs` files (see Rust core rules above).
- **File size**: prefer ≤ ~500 lines per source file; split modules when growing.
- **Pre-merge checks** (when touching code): Prettier, ESLint, `tsc --noEmit` in `app/`; `cargo fmt` + `cargo check` for changed Rust.
- **No dynamic imports** in production **`app/src`** code — use static `import` / `import type` at the top of the module. Do **not** use `import()`, `React.lazy(() => import(...))`, or `await import('…')`. Guard optional code paths with `try/catch` or runtime checks instead of deferring module load. **Exceptions:** Vitest harness patterns in `*.test.ts` / `test/setup.ts`; ambient `typeof import('…')` in `.d.ts`; config files.
- **Debug logging on new flows**: heavy `debug`/`trace` (Rust) and namespaced `debug` / dev logs (`app/`) so sidecar + WebView output is easy to grep. Never log secrets or raw tokens. Full rule: [`docs/feature-workflow.md`](docs/feature-workflow.md).
- **Dual socket / tool sync**: if you change realtime protocol, keep **frontend** (`socketService` / MCP transport) and **core** socket behavior aligned (see [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) dual-socket section).

---

_Last aligned with monorepo layout (`app/` + root `src/`), QuickJS skills in `openhuman_core`, skills catalog on GitHub (`tinyhumansai/openhuman-skills`), and Tauri shell IPC as of repo state._
