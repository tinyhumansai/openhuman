# OpenHuman

**AI-powered assistant for crypto communities — React + Tauri v2 desktop app with a Rust core (JSON-RPC / CLI) and sandboxed QuickJS skills.**

This file orients contributors and coding agents. Authoritative narrative architecture: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). Frontend layout: [`docs/src/README.md`](docs/src/README.md). Tauri shell: [`docs/src-tauri/README.md`](docs/src-tauri/README.md).

---

## Repository layout

| Path                    | Role                                                                                                                                                                                                      |
| ----------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **`app/`**              | Yarn workspace **`openhuman-app`**: Vite + React (`app/src/`), Tauri desktop host (`app/src-tauri/`), Vitest tests                                                                                        |
| **Repo root `src/`**    | Rust library **`openhuman_core`** and **`openhuman`** CLI binary (`src/bin/openhuman.rs`) — `core_server`, `openhuman::*` domains, skills runtime (QuickJS / `rquickjs`), MCP routing in the core process |
| **`skills/`**           | Skill packages (built into `skills/skills` for bundling)                                                                                                                                                  |
| **`Cargo.toml`** (root) | Core crate; `cargo build --bin openhuman` produces the sidecar the UI stages via `app`’s `core:stage`                                                                                                     |
| **`docs/`**             | Architecture and module guides (numbered pages under `docs/src/`, `docs/src-tauri/`)                                                                                                                      |

Commands in documentation assume the **repo root** unless noted: `yarn dev` runs the `app` workspace.

---

## Runtime scope

- **Shipped product**: desktop — Windows, macOS, Linux (see [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) “Platform reach”).
- **Tauri host** (`app/src-tauri`): **desktop-only** (`compile_error!` for non-desktop targets). Do not add Android/iOS branches inside `app/src-tauri`.
- **Core binary** (`openhuman`): spawned/staged as a **sidecar**; the Web UI talks to it over HTTP (`core_rpc_relay` + `core_rpc` client), not by re-implementing domain logic in the shell.

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

# Skills (under skills/)
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
- **Light `mod.rs` rule**: Keep domain `mod.rs` files light and export-focused. Put operational code in sibling files (example: `ops.rs`, `store.rs`, `schedule.rs`, `types.rs`), then re-export the public API from `mod.rs`.
- **`core_server/`** — Transport only: Axum/HTTP, JSON-RPC envelope, CLI parsing, **dispatch** (`core_server::dispatch`) — **no** heavy business logic here.
- **Layering**: Implementation in `openhuman::<domain>/`, controllers in `openhuman::<domain>/rpc.rs`, routes in `core_server/`.

Skills runtime uses **QuickJS** (`rquickjs`) in **`src/openhuman/skills/`** (e.g. `qjs_skill_instance.rs`, `qjs_engine.rs`), not V8/deno_core in this repository.

---

## App theme & design system

**Design intent**: Premium, calm crypto UI — ocean primary (`#4A83DD`), sage / amber / coral semantic colors, Inter + Cabinet Grotesk + JetBrains Mono, Tailwind with custom radii/spacing/shadows. Details: [`docs/DESIGN_GUIDELINES.md`](docs/DESIGN_GUIDELINES.md).

---

## Environment variables (Vite)

Set in `.env` for the **`app`** workspace (`VITE_*` exposed to the client):

| Variable                  | Purpose                                                                   |
| ------------------------- | ------------------------------------------------------------------------- |
| `VITE_BACKEND_URL`        | API base (default in code: production API; see `app/src/utils/config.ts`) |
| `VITE_SENTRY_DSN`         | Optional Sentry DSN                                                       |
| `VITE_SKILLS_GITHUB_REPO` | Skills catalog GitHub repo slug                                           |

---

## Git workflow

- **Public repo**; push to your working branch; PRs target **`main`**.
- Use [`.github/pull_request_template.md`](.github/pull_request_template.md); AI-generated PR text should follow its sections and checklist.

---

## Key patterns (concise)

- **`src/openhuman/`**: New features go in a **folder/module**, not new root-level `src/openhuman/*.rs` files (see Rust core section).
- **File size**: Prefer ≤ ~500 lines per source file; split modules when growing.
- **Pre-merge checks** (when touching code): Prettier, ESLint, `tsc --noEmit` in `app/`; `cargo fmt` + `cargo check` for changed Rust (`Cargo.toml` at root and/or `app/src-tauri/Cargo.toml` as appropriate).
- **No dynamic imports** in app code (static `import` only); use try/catch around Tauri APIs where needed.
- **Type-only imports**: `import type` where appropriate.
- **Dual socket / tool sync**: If you change realtime protocol, keep **frontend** (`socketService` / MCP transport) and **core** socket behavior aligned (see [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) dual-socket section).

---

## Platform notes

- **macOS deep links**: Often require a built **`.app`** bundle; not only `tauri dev`. See [`docs/telegram-login-desktop.md`](docs/telegram-login-desktop.md) if applicable.
- **`window.__TAURI__`**: Not assumed at module load; guard Tauri usage accordingly.
- **Core sidecar**: Must be staged/built so `core_rpc` can reach the `openhuman` binary (see `scripts/stage-core-sidecar.mjs`).

---

_Last aligned with monorepo layout (`app/` + root `src/`), QuickJS skills in `openhuman_core`, and Tauri shell IPC as of repo state._
