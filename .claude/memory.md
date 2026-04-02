# Project Memory

Quick reference for anyone starting with Claude on this project. Updated by the `memory-keeper` agent.

## Fixes & Gotchas

- **ServiceBlockingGate CORS errors** — The gate calls `openhumanServiceStatus()` and `openhumanAgentServerStatus()` at startup. These used `callCoreRpc()` which falls back to raw `fetch()` when socket isn't connected yet, causing CORS errors. Fix: route through `invoke('core_rpc_relay')` instead (Tauri IPC, no CORS).
- **Socket not connected at startup** — `SocketProvider` only connects when a Redux `auth.token` is set. At fresh launch (no token), socket is null, so any `callCoreRpc()` call falls back to `fetch()`. Always use `invoke('core_rpc_relay')` for local sidecar RPC calls.
- **`openhuman.agent_server_status` doesn't exist** — This RPC method is not registered in the core. The gate checks it but it always errors. The gate passes if either service is Running OR agent server is running OR core is reachable.
- **Cargo incremental builds can serve stale UI** — If the app shows old frontend after a Rust rebuild, run `cargo clean --manifest-path app/src-tauri/Cargo.toml` before rebuilding.
- **macOS deep links require .app bundle** — `yarn tauri dev` does NOT support deep links. Must use `yarn tauri build --debug --bundles app`.

## Strict Rules

- **No dynamic imports in `app/src/`** — Use static `import` at file top. Guard call sites with `try/catch` for Tauri/non-Tauri safety. See CLAUDE.md.
- **Service RPC calls must use Tauri IPC** — Never use `callCoreRpc()` for service operations. Use `invoke('core_rpc_relay', { request: { method, params } })`.
- **All frontend env vars go through `app/src/utils/config.ts`** — Never read `import.meta.env.VITE_*` directly in other files. Import from config.ts instead. See `.env.example` files for the full list.
- **Always run checks before commit** — `yarn typecheck`, `yarn lint`, `yarn format:check`, `yarn build`, `yarn tauri dev`. Husky hooks enforce some but run all manually first.
- **Stage specific files** — Never `git add -A`. Always `git add <specific-files>`.

## Workflow

- **Agent order**: architectobot (plan) → user approval → codecrusher (implement) → architectobot (verify)
- **Always read CLAUDE.md first** before any issue work
- **Ask user when in doubt** — never assume scope or approach
- **PRs target upstream** — `tinyhumansai/openhuman` main branch, not fork

## Local AI Presets & Daemon Gotcha

- **Tier system lives in `src/openhuman/local_ai/presets.rs`** — single source of truth for tier→model ID mapping. To change default models for a release, edit `all_presets()` there.
- **Device detection** uses `sysinfo` crate (`src/openhuman/local_ai/device.rs`). Apple Silicon = GPU always; others = best-effort.
- **`OPENHUMAN_LOCAL_AI_TIER` env var** overrides the selected tier at config load time (in `load.rs`).
- **Frontend tier selector** is in `LocalModelPanel.tsx` under Settings > Local AI Model. Uses `coreRpcClient` to call 3 RPC methods: `local_ai_device_profile`, `local_ai_presets`, `local_ai_apply_preset`.
- **Default config maps to Medium tier** (`gemma3:4b-it-qat`). If someone changes `model_ids.rs` defaults, they should keep `presets.rs` in sync.
- **Daemon binary gotcha** — A daemon process (`openhuman-aarch64-apple-darwin run`) auto-starts on port 7788 and respawns on kill. `yarn tauri dev` reuses it if already running. When adding new RPC methods, you must replace this binary: `cp -f target/debug/openhuman-core app/src-tauri/binaries/openhuman-aarch64-apple-darwin`, then kill the old PID so it respawns with the new binary.

## Onboarding System

- **OnboardingOverlay is a portal, not a route** — mounted in `App.tsx`, renders via `createPortal` at z-[9999]. There is no `/onboarding` route in `AppRoutes.tsx`. Gating is purely Redux + workspace flag.
- **Deferred onboarding** — `onboardingDeferredByUser` in `authSlice.ts` (persisted via redux-persist) durably tracks when a user clicks "Set up later". `SetupBanner.tsx` provides the resume path.
- **`selectHasIncompleteOnboarding` is unused** in production code — only tested. Don't use it for new features.
- **Logout must clear onboarding state** — `_clearToken` resets `isOnboardedByUser` + `isAnalyticsEnabledByUser`. Workspace flag (`.skip_onboarding` file) is cleared via `openhumanWorkspaceOnboardingFlagSet(false)` in SettingsHome logout, clearAllAppData, and UserProvider auth recovery. All three paths must stay in sync. **OnboardingOverlay local state** (`userLoadTimedOut`, `onboardingCompleted`) is reset via a `useEffect` watching `token` — if `token` becomes null, both reset to initial values (#192).
- **`formatBytes` / `formatEta` / `progressFromStatus`** — shared in `app/src/utils/localAiHelpers.ts`. Home.tsx and LocalModelPanel.tsx still have local copies (can be migrated later).
- **Notification z-index stacking** — ErrorReportNotification: z-[10000] bottom-right. OnboardingOverlay: z-[9999]. LocalAIDownloadSnackbar: z-[9998] bottom-left.
- **React Compiler lint** — `useCallback` deps must match the full inferred closure. Using `user?._id` as dep when the closure captures `user` triggers `preserve-manual-memoization`. Use `user` as the dep instead.
- **`setState` in effects** — ESLint `react-hooks/set-state-in-effect` catches synchronous setState in useEffect bodies. Use lazy initializers, compute at render, or event handlers instead.

## Environment

- **Core sidecar port** — `7788` (default). Check with `lsof -i :7788`.
- **Stage sidecar** — `cd app && yarn core:stage` (required for core RPC).
- **Kill stuck processes** — `lsof -i :7788` then `kill <PID>`.
