# Skills: How They Work End-to-End

This document explains how OpenHuman skills are discovered, fetched, installed, initialized, executed, and synchronized across the desktop app and Rust core.

It is written for engineers who need to debug, extend, or migrate the skills system.

---

## 1) Mental Model

OpenHuman has two skill-related paths:

1. **Active runtime path (authoritative for execution):**
   - QuickJS skills managed by Rust core runtime.
   - Accessed through JSON-RPC methods under `openhuman.skills_*`.
   - UI acts as an RPC client and orchestration layer.

2. **Legacy metadata path (still present):**
   - Workspace scanning of `skill.json` + `SKILL.md`.
   - Used for older prompt/context loading flows, not the primary execution runtime.

If you are implementing runtime behavior, use the active QuickJS path.

---

## 2) Key Directories and Files

### Frontend (app)

- `app/src/lib/skills/skillsApi.ts`
  - Typed RPC wrapper for skills methods (`list_available`, `install`, `start`, `rpc`, etc.).
- `app/src/lib/skills/manager.ts`
  - Orchestrates setup, OAuth completion, tool usage, and sync triggers.
- `app/src/lib/skills/runtime.ts`
  - Runtime-facing wrapper around skill lifecycle/tool calls.
- `app/src/lib/skills/hooks.ts`
  - Read hooks for snapshots and available skills.
- `app/src/lib/skills/sync.ts`
  - Maps snapshots to tool sync payloads.
- `app/src/lib/skills/skillEvents.ts`
  - Event emitter for local invalidation/re-fetch.
- `app/src/utils/desktopDeepLinkListener.ts`
  - Handles deep links (including OAuth complete/error) and notifies runtime.
- `app/src/utils/config.ts`
  - Frontend config values including `VITE_SKILLS_GITHUB_REPO`.

### Rust core

- `src/core/jsonrpc.rs`
  - Core server startup and runtime bootstrap (`bootstrap_skill_runtime`).
- `src/openhuman/skills/schemas.rs`
  - Controller schemas and handlers for `openhuman.skills_*` methods.
- `src/openhuman/skills/registry_ops.rs`
  - Remote registry fetch/cache/search/install/uninstall/list logic.
- `src/openhuman/skills/registry_types.rs`
  - Registry and available/installed type shapes.
- `src/openhuman/skills/qjs_engine.rs`
  - Runtime engine for discovery/start/stop/rpc/tool execution.
- `src/openhuman/skills/manifest.rs`
  - Manifest parsing and platform/runtime eligibility checks.
- `src/openhuman/skills/skill_registry.rs`
  - Running skill registry, message routing, snapshots.
- `src/openhuman/skills/qjs_skill_instance/*`
  - QuickJS instance lifecycle, event loop, JS handlers.
- `src/openhuman/skills/quickjs_libs/bootstrap.js`
  - JS environment bootstrap and bridged APIs.
- `src/openhuman/skills/socket_manager.rs`
  - Socket integration for tool sync and tool-call routing.
- `src/openhuman/skills/preferences.rs`
  - Persisted per-skill preference state (enabled/setup flags).

### Legacy path (non-authoritative for runtime execution)

- `src/openhuman/skills/ops.rs`
  - `workspace/skills` scanner for `skill.json` and `SKILL.md`.

---

## 3) Skill Packaging and Storage

The active runtime expects each skill directory to contain at minimum:

- `manifest.json`
- JS entry file (usually `index.js`, but depends on manifest `entry`)

Installed skills are written to:

- `${workspace_dir}/skills/<skill_id>/manifest.json`
- `${workspace_dir}/skills/<skill_id>/<entry_file>`

The runtime also has a skills data area:

- `${base_dir}/skills_data/<skill_id>/...`

Where:

- `base_dir` is `$OPENHUMAN_WORKSPACE` if set, otherwise `~/.openhuman`
- `workspace_dir` is `${base_dir}/workspace`

---

## 4) Registry Fetch and Availability Flow

### Registry source

The core fetches a JSON registry from:

- `SKILLS_REGISTRY_URL` if set, else
- default:
  `https://raw.githubusercontent.com/tinyhumansai/openhuman-skills/refs/heads/build/skills/registry.json`

### Caching

The registry is cached to:

- `${workspace_dir}/skills/.registry-cache.json`

Cache TTL is one hour.

### Availability API

When UI calls `openhuman.skills_list_available`, core:

1. Fetches or reads cached registry.
2. Scans installed skill directories under `workspace/skills`.
3. Merges both views:
   - `installed` boolean
   - `installed_version`
   - `update_available`

### Install API

When UI calls `openhuman.skills_install`, core:

1. Finds skill entry by ID in registry.
2. Downloads `manifest_url` and `download_url`.
3. Verifies checksum if `checksum_sha256` exists.
4. Writes files under `workspace/skills/<skill_id>/`.

Uninstall removes that directory.

---

## 5) Runtime Bootstrap and Auto-Start

On core startup, `bootstrap_skill_runtime()`:

1. Resolves `base_dir`.
2. Creates `skills_data` directory.
3. Creates `RuntimeEngine`.
4. Sets `workspace_dir` on engine (`<base_dir>/workspace`).
5. Registers engine globally for RPC handlers.
6. Starts ping and cron schedulers.
7. Launches async auto-start.

Auto-start behavior is driven by:

- discovered manifests (`discover_skills`)
- manifest defaults (`auto_start`)
- preference overrides (`enable/disable` and setup state persistence)

---

## 6) Discovery and Start Rules

`discover_skills()` scans two locations:

1. Runtime source directory (bundled/dev source path resolution).
2. Workspace installed directory (`workspace/skills`).

For each candidate:

- Reads `manifest.json`
- Requires JavaScript runtime compatibility
- Checks current platform compatibility
- Deduplicates by `manifest.id`

`start_skill(skill_id)` behavior:

1. Returns existing running/initializing snapshot if already active.
2. Resolves directory (source dir first, workspace fallback).
3. Validates manifest runtime/platform.
4. Creates a QuickJS skill instance.
5. Spawns event loop and registers skill in registry.
6. Runs lifecycle (`init`, then `start`).
7. Exposes current snapshot/tools/state.

---

## 7) Runtime Message Model

Most interactions become messages from engine to skill instance event loop.

Typical operations:

- `start` / `stop`
- generic rpc (`openhuman.skills_rpc`)
- tool call (`openhuman.skills_call_tool`)
- setup events (`setup/start`, `oauth/complete`)
- sync/tick events (`skill/tick`)

Tool calls can be sync or async in JS. Async calls are awaited with runtime polling and timeout handling in the QuickJS event loop layer.

---

## 8) JSON-RPC Surface (`openhuman.skills_*`)

The skills controllers are registered in `src/openhuman/skills/schemas.rs`.

Current method families:

- Registry/catalog:
  - `openhuman.skills_registry_fetch`
  - `openhuman.skills_search`
  - `openhuman.skills_list_available`
  - `openhuman.skills_list_installed`
  - `openhuman.skills_install`
  - `openhuman.skills_uninstall`
- Runtime lifecycle/state:
  - `openhuman.skills_discover`
  - `openhuman.skills_list`
  - `openhuman.skills_start`
  - `openhuman.skills_stop`
  - `openhuman.skills_status`
  - `openhuman.skills_get_all_snapshots`
- Runtime actions:
  - `openhuman.skills_list_tools`
  - `openhuman.skills_call_tool`
  - `openhuman.skills_rpc`
  - `openhuman.skills_sync`
  - `openhuman.skills_setup_start`
- Persistence/control:
  - `openhuman.skills_enable`
  - `openhuman.skills_disable`
  - `openhuman.skills_is_enabled`
  - `openhuman.skills_set_setup_complete`
  - `openhuman.skills_data_read`
  - `openhuman.skills_data_write`
  - `openhuman.skills_data_dir`

---

## 9) OAuth and Setup Completion Flow (Desktop)

OAuth callback is handled in `desktopDeepLinkListener.ts`.

For `openhuman://oauth/success?...`:

1. Persist setup complete via `openhuman.skills_set_setup_complete`.
2. Ensure skill is running via `openhuman.skills_start`.
3. Send `oauth/complete` via `openhuman.skills_rpc`.
4. Trigger initial sync (`skillManager.triggerSync`).
5. Emit local skill-state refresh event.

This keeps persistence, runtime, and UI in sync after browser-based auth.

---

## 10) State and Snapshot Model

Skill state can be published from JS via bridge APIs (`state.*` in bootstrap environment).

Core tracks snapshots containing:

- skill id/name/status
- tools
- runtime error (if any)
- published state map
- setup and connection status

Frontend hooks (`useSkillSnapshot`, `useAllSkillSnapshots`, etc.) render from these snapshots and refresh on skill events.

---

## 11) Tool Sync and Socket Integration

Socket manager bridges runtime tool inventory and MCP-style calls.

High-level pattern:

1. Core publishes available tools from running skills.
2. Frontend/runtime sync maps snapshots to tool payload.
3. Incoming tool calls route to `skill_id` + `tool_name`.
4. Core executes via runtime and returns `ToolResult`.

---

## 12) Environment Variables and Configuration

### Core/runtime relevant

- `SKILLS_REGISTRY_URL`
  - Override skill catalog URL.
- `OPENHUMAN_WORKSPACE`
  - Sets base workspace root (`skills_data`, `workspace/skills`, config).
- `OPENHUMAN_CORE_PORT`
  - Core JSON-RPC HTTP port.
- `OPENHUMAN_CORE_RUN_MODE`
  - Tauri core launch mode behavior.
- `OPENHUMAN_CORE_BIN`
  - Override core binary path.

### Frontend relevant

- `VITE_SKILLS_GITHUB_REPO`
  - UI-side repository slug default for skills registry context/display.
  - Note: runtime fetch authority is still `SKILLS_REGISTRY_URL` in core.
- `VITE_OPENHUMAN_CORE_RPC_URL` / `OPENHUMAN_CORE_RPC_URL`
  - Core RPC endpoint override for app client.

---

## 13) End-to-End Sequence (Install + OAuth + Tool Call)

1. User opens Skills screen.
2. UI calls `openhuman.skills_list_available`.
3. Core returns registry + installed/enriched availability.
4. User clicks install/connect.
5. UI calls `openhuman.skills_install`.
6. UI starts skill via `openhuman.skills_start`.
7. Skill initializes in QuickJS (`init` then `start`).
8. OAuth browser completes and deep links back.
9. UI marks setup complete, sends `oauth/complete`, triggers sync.
10. Agent or UI calls tool.
11. Core routes tool call to skill event loop and returns result.

---

## 14) Debugging Guide

### Common checks

1. Registry errors:
   - verify `SKILLS_REGISTRY_URL`
   - inspect cache file under `workspace/skills/.registry-cache.json`
2. Install issues:
   - check `manifest_url`/`download_url` accessibility
   - validate checksum mismatch logs
3. Startup issues:
   - ensure `manifest.json` exists and `runtime` is JS-compatible
   - verify platform filter in manifest
4. OAuth issues:
   - confirm deep-link callback includes `integrationId` and `skillId`
   - verify `set_setup_complete` and `oauth/complete` RPCs are invoked
5. Tool-call failures:
   - verify skill status is `Running`
   - inspect skill error in snapshot

### Useful runtime truths

- Catalog truth: remote registry (+ cache)
- Installed truth: `workspace/skills/*/manifest.json`
- Running truth: runtime snapshots from `openhuman.skills_status` / `openhuman.skills_get_all_snapshots`

---

## 15) Known Split-Brain Risks

There is an intentional but risky overlap between:

- QuickJS runtime manifests (`manifest.json`) and
- legacy loader semantics (`skill.json` + `SKILL.md`)

Impact:

- Different subsystems can report different views of "what skills exist."
- Documentation or migration work can accidentally target the wrong system.

Recommendation:

- Treat `openhuman.skills_*` + QuickJS manifests as canonical for execution paths.
- Keep legacy path use explicitly scoped until fully migrated.

---

## 16) Testing Coverage Pointers

- Registry/install/runtime e2e validations:
  - `tests/json_rpc_e2e.rs`
- Core unit tests:
  - `src/openhuman/skills/*` (registry/runtime modules)
- App integration points:
  - `app/src/lib/skills/*`
  - deep-link flow in `app/src/utils/desktopDeepLinkListener.ts`

When changing behavior, test both:

1. JSON-RPC behavior from core (`openhuman.skills_*` methods)
2. App orchestration behavior (especially OAuth/setup/sync)

---

## 17) Practical Rules for Contributors

- Put business/runtime behavior in Rust core.
- Keep frontend as orchestration and UX.
- Prefer adding/using explicit `openhuman.skills_*` methods over side channels.
- Preserve setup + enabled flags coherently across restarts.
- Avoid introducing new legacy skill metadata paths.
- Add traceable logs around install/start/setup/tool call boundaries.

