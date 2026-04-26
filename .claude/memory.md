# Project Memory

Quick reference for anyone starting with Claude on this project. Updated by the `memory-keeper` agent.

## Fixes & Gotchas

- **ServiceBlockingGate CORS errors** — The gate calls `openhumanServiceStatus()` and `openhumanAgentServerStatus()` at startup. These used `callCoreRpc()` which falls back to raw `fetch()` when socket isn't connected yet, causing CORS errors. Fix: route through `invoke('core_rpc_relay')` instead (Tauri IPC, no CORS).
- **Socket not connected at startup** — `SocketProvider` only connects when a Redux `auth.token` is set. At fresh launch (no token), socket is null, so any `callCoreRpc()` call falls back to `fetch()`. Always use `invoke('core_rpc_relay')` for local sidecar RPC calls.
- **`openhuman.agent_server_status` doesn't exist** — This RPC method is not registered in the core. The gate checks it but it always errors. The gate passes if either service is Running OR agent server is running OR core is reachable.
- **Cargo incremental builds can serve stale UI** — If the app shows old frontend after a Rust rebuild, run `cargo clean --manifest-path app/src-tauri/Cargo.toml` before rebuilding.
- **`build.rs` missing `rerun-if-changed` causes stale ACL / "Command not found" at runtime** — `app/src-tauri/build.rs` had no `cargo:rerun-if-changed` directives for `permissions/` or `capabilities/`. Adding/changing TOML or JSON files there did not re-trigger `tauri-build`, so ACL tables were stale and registered commands silently failed. Fixed by adding `println!("cargo:rerun-if-changed=permissions")` and `println!("cargo:rerun-if-changed=capabilities")` in `build.rs` (issue #270). Also: any new Tauri command must have a matching entry in a `permissions/` TOML file or it will hit the same error even if it is in `generate_handler!`.
- **macOS deep links require .app bundle** — `pnpm tauri dev` does NOT support deep links. Must use `pnpm tauri build --debug --bundles app`.

## Strict Rules

- **No dynamic imports in `app/src/`** — Use static `import` at file top. Guard call sites with `try/catch` for Tauri/non-Tauri safety. See CLAUDE.md.
- **Service RPC calls must use Tauri IPC** — Never use `callCoreRpc()` for service operations. Use `invoke('core_rpc_relay', { request: { method, params } })`.
- **All frontend env vars go through `app/src/utils/config.ts`** — Never read `import.meta.env.VITE_*` directly in other files. Import from config.ts instead. See `.env.example` files for the full list.
- **Always run checks before commit** — `pnpm typecheck`, `pnpm lint`, `pnpm format:check`, `pnpm build`, `pnpm tauri dev`. Husky hooks enforce some but run all manually first.
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
- **Daemon binary gotcha** — A daemon process (`openhuman-aarch64-apple-darwin run`) auto-starts on port 7788 and respawns on kill. `pnpm tauri dev` reuses it if already running. When adding new RPC methods, you must replace this binary: `cp -f target/debug/openhuman-core app/src-tauri/binaries/openhuman-aarch64-apple-darwin`, then kill the old PID so it respawns with the new binary.

## Onboarding System

- **OnboardingOverlay is a portal, not a route** — mounted in `App.tsx`, renders via `createPortal` at z-[9999]. There is no `/onboarding` route in `AppRoutes.tsx`. Gating is purely Redux + workspace flag.
- **Deferred onboarding** — `onboardingDeferredByUser` in `authSlice.ts` (persisted via redux-persist) durably tracks when a user clicks "Set up later". `SetupBanner.tsx` provides the resume path.
- **`selectHasIncompleteOnboarding` is unused** in production code — only tested. Don't use it for new features.
- **Logout must clear onboarding state** — `_clearToken` resets `isOnboardedByUser` + `isAnalyticsEnabledByUser`. Workspace flag (`.skip_onboarding` file) is cleared via `openhumanWorkspaceOnboardingFlagSet(false)` in SettingsHome logout, clearAllAppData, and UserProvider auth recovery. All three paths must stay in sync. **OnboardingOverlay local state** (`userLoadTimedOut`, `onboardingCompleted`) is reset via a `useEffect` watching `token` — if `token` becomes null, both reset to initial values (#192).
- **LocalAI download errors must surface** — `LocalAIStep` has an `onDownloadError` callback prop; `Onboarding.tsx` renders an error banner via `createPortal` when it fires. Without this, download failures are silently swallowed (#194).
- **`formatBytes` / `formatEta` / `progressFromStatus`** — shared in `app/src/utils/localAiHelpers.ts`. Home.tsx and LocalModelPanel.tsx still have local copies (can be migrated later).
- **Notification z-index stacking** — ErrorReportNotification: z-[10000] bottom-right. OnboardingOverlay: z-[9999]. LocalAIDownloadSnackbar: z-[9998] bottom-left.
- **React Compiler lint** — `useCallback` deps must match the full inferred closure. Using `user?._id` as dep when the closure captures `user` triggers `preserve-manual-memoization`. Use `user` as the dep instead.
- **`setState` in effects** — ESLint `react-hooks/set-state-in-effect` catches synchronous setState in useEffect bodies. Use lazy initializers, compute at render, or event handlers instead.
- **`OnboardingNextButton` is the shared primary CTA** — All onboarding steps use `app/src/pages/onboarding/components/OnboardingNextButton.tsx`. New steps must use this component for the primary navigation button.
- **Onboarding is 3 steps: Welcome(0) → Skills(1) → ContextGathering(2)** — Referral step was removed (issue #752). `ReferralApplyStep.tsx` is preserved but unused. `referralApi` is still used on the Rewards page. `WelcomeStep` no longer has `nextDisabled`/`nextLoading`/`nextLoadingLabel` props (those gated on referral stats prefetch).
- **Recovery Phrase moved to Settings** — MnemonicStep was removed from onboarding (was step 5). The same BIP39 generate/import functionality now lives in `app/src/components/settings/panels/RecoveryPhrasePanel.tsx`, accessible via Settings > Recovery Phrase. Onboarding completion logic moved into `handleSkillsNext` in `Onboarding.tsx`.
- **E2E tests find onboarding buttons by label text** — `shared-flows.ts`, `login-flow.spec.ts`, `auth-access-control.spec.ts`, and `voice-mode.spec.ts` locate buttons by their visible label. Changing button labels requires updating all four files. Note: `voice-mode.spec.ts` still references legacy labels that don't match current steps (pre-existing tech debt).
- **`ScreenPermissionsStep` always shows Continue** — The Continue button is always visible regardless of permission grant status, allowing users to skip the permissions step (#274).
- **OnboardingOverlay RPC/Redux race condition** — `getOnboardingCompleted()` RPC can fail (sidecar not ready, timeout); the old catch block hardcoded `setOnboardingCompleted(false)`, ignoring the persisted `isOnboardedByUser` Redux flag. Fix: read `selectIsOnboarded` from `authSelectors.ts` in the catch block as fallback, and combine both flags in `shouldShow`: `!onboardingCompleted && !isOnboardedRedux`. Either flag being `true` is sufficient to skip onboarding (#197).
- **`DEV_FORCE_ONBOARDING` was a no-op** — The old ternary had identical branches; fixed to actually force-show when the flag is set.
- **`isOnboardedRedux` must be in useEffect deps** — When reading a selector value inside a useEffect, add it to the dependency array or the effect won't re-run when Redux state changes.

## CoreStateProvider & Auth Bootstrap

- **Auth session tokens are NOT in Redux persist** — They live entirely in the Rust sidecar, fetched via `fetchCoreAppSnapshot()` RPC. `PersistGate` only gates non-auth state (AI config, threads, channel connections). `CoreStateProvider` bootstrap is the critical auth path.
- **`CoreStateProvider` premature `isBootstrapping: false` causes blank Settings** — If the initial RPC call fails (sidecar still starting), the old error handler set `isBootstrapping: false` immediately, causing `ProtectedRoute` to redirect to `/` before the 3s poll could recover. Fix (issue #413): keep `isBootstrapping: true` on initial failure, let the poll retry, give up after 5 attempts (~15s).
- **`CoreStateProvider` is consumed by ~25 components** — Changes to its state shape or bootstrap behavior affect routes, socket, onboarding, nav, settings, and hooks. Treat it as a high-blast-radius file.
- **Settings is a full route, not a modal** — `/settings/*` uses nested `<Routes>` in `Settings.tsx`. The `.claude/rules/15-settings-modal-system.md` doc describing a portal/modal approach is outdated. A catch-all `<Route path="*">` redirects unmatched sub-paths to `/settings`.
- **`PersistGate loading={null}` causes flash** — Changed to `loading={<RouteLoadingScreen />}` (issue #413). `RouteLoadingScreen` accepts an optional `label` prop (defaults to "Initializing OpenHuman...") and can be rendered with no props.

## Build Blockers: macOS Tahoe + whisper-rs

- **`whisper-rs` breaks `cargo build` on macOS Tahoe (Apple Silicon)** — Added in main via `whisper-rs = "0.16"` (voice feature #178). Apple clang 21+ refuses `-mcpu=native` when `--target=arm64-apple-macosx` is also set. This is NOT fixable by updating CLT.
- **Root cause** — ggml cmake sets `GGML_NATIVE=ON` by default; the cmake crate appends `--target` to clang, triggering the incompatibility. Happens even with the latest toolchain.
- **Workaround** — Patch `~/.cargo/registry/src/index.crates.io-*/whisper-rs-sys-0.15.0/build.rs`: add `config.define("GGML_NATIVE", "OFF");` (for `target_os = "macos" && target_arch = "aarch64"`) just before the `config.build()` call.
- **Patch is fragile** — Resets on `cargo clean`, crate version bump, or registry re-download. Deleting build cache alone (`target/debug/build/whisper-rs-sys-*`) is NOT enough — cmake regenerates with the same bad flags.
- **Correct fix** — Needs an upstream patch in `whisper-rs-sys` or a Cargo feature to opt out of `GGML_NATIVE` on Apple Silicon cross-builds.

## UI Redesign (Light Theme — April 2026)

- **Full dark-to-light redesign shipped** — All pages, components, and settings panels converted from dark glass-morphism to clean light theme based on Figma designs by Mithil (`OpenHuman-Prod` file, node `2094-250136` for tokens).
- **Design tokens saved** in `my_docs/figma-design-tokens.md` — neutral grayscale, primary blue `#2F6EF4`, success `#34C759`, alert `#E8A728`, error `#EF4444`, SF Pro typography scale.
- **Navigation changed**: Left `MiniSidebar` → bottom `BottomTabBar` (Home, Chat, Skills, Intelligence, Automation, Notification). Settings accessible via gear icon on Home page header.
- **MiniSidebar.tsx retained** (not deleted) as backup. `BottomTabBar.tsx` is the active nav component.
- **Agent message bubbles** need `bg-stone-200/80` (not `bg-stone-100`) on `#F5F5F5` background — `bg-stone-100` is nearly invisible.
- **~55 files touched** — purely CSS class changes, zero logic/handler/state changes.

## Upsell / Billing (Phase 1 — Issue #403)

- **Upsell components** live in `app/src/components/upsell/` — `UpsellBanner`, `UsageLimitModal`, `GlobalUpsellBanner`, `upsellDismissState`. Shared hook: `app/src/hooks/useUsageState.ts`.
- **Usage data sources** — `creditsApi.getTeamUsage()` returns `TeamUsage` (rolling 10h spend/cap + weekly budget/remaining). `billingApi.getCurrentPlan()` returns `CurrentPlanData` (plan tier, caps, subscription status). Both go through `callCoreCommand` (core RPC). No Redux slice — all local hook state.
- **Module-level cache in `useUsageState`** — `_cache` variable with 60s TTL prevents duplicate API calls when multiple components mount simultaneously. New pattern; do not remove.
- **Banner dismiss state uses localStorage** (prefix `openhuman:upsell:`), not Redux — consistent with CLAUDE.md exception for ephemeral UI state.
- **Phased rollout** — Phase 1 = banners + limit modal + hook. Phase 2 = onboarding upsell + analytics. Phase 3 = remote config + A/B testing.
- **"5-hour" label stragglers in Conversations.tsx** — `LimitPill` label and its hover tooltip still say "5h" / "5-hour". Commit 8c52236's "10-hour" terminology refactor missed those two spots.
- **`getTeamUsage()` now normalizes via `normalizeTeamUsage()`** — Added in issue #482. The Rust sidecar passes backend JSON through opaquely (`src/openhuman/team/ops.rs`), so the TS client must normalize field names and types. Pattern matches existing `normalizeCreditBalance()` in the same file. Any new billing API that returns raw backend data should follow the same normalize-at-the-client pattern.
- **Two separate `TeamUsage` types exist** — `creditsApi.ts:24` (billing: cycle budget, limits) and `types/team.ts:11` (team model: daily token limit). Different import paths, no collision, but confusing.

## Settings & Skills Reorganization (Issue #396)

- **Settings is NOT a modal** — It's a full route (`/settings/*`) with nested `<Routes>`. The `.claude/rules/15-settings-modal-system.md` doc is outdated.
- **SettingsHeader breadcrumbs** — All panels now receive `breadcrumbs` from `useSettingsNavigation()` hook. The hook derives breadcrumbs from the current route path. When adding a new settings panel, destructure `breadcrumbs` from the hook and pass to `<SettingsHeader>`.
- **Standard settings padding** — All settings panel content areas use `p-4 space-y-4`. Don't deviate.
- **Dead code removed** — `TauriCommandsPanel`, `useSettingsAnimation`, `SettingsPanelLayout`, `SettingsBackButton`, `ProfilePanel`, `AdvancedPanel`, `SkillsPanel`, `SkillsGrid` were all deleted. Don't re-create them.
- **Skills page is the single management surface** — Browser Access toggle moved from SkillsPanel to the Skills page. There is no `/settings/skills` route anymore.
- **Panel decomposition** — LocalModelPanel, AutocompletePanel, CronJobsPanel, ScreenIntelligencePanel were split into sub-components in subdirectories. Each orchestrator is ≤ ~300 lines.
- **UnifiedSkillCard** — All skill types (built-in, channels, 3rd party) use `UnifiedSkillCard` from `app/src/components/skills/SkillCard.tsx`. Secondary actions use an overflow menu. `data-testid` attributes (`skill-sync-button-*`, `skill-debug-button-*`) must be preserved.
- **SkillSearchBar + SkillCategoryFilter** — New components in `app/src/components/skills/` for search and category filtering on the Skills page.

## Composio Identity (Issue #691)

- **`ProviderUserProfile.profile_url`** — New optional field on the struct in `src/openhuman/composio/providers/types.rs`. Providers should populate it when available from upstream profile payloads.
- **`identity_set` callback in default flow** — `ComposioProvider::on_connection_created()` in `src/openhuman/composio/providers/traits.rs` now calls `identity_set(&profile)` after profile fetch. `composio_get_user_profile` in `src/openhuman/composio/ops.rs` also routes persistence through `identity_set`.
- **Facet key format for connected identities** — `skill:{toolkit}:{identifier}:{field}` (e.g. `skill:gmail:user@example.com:profile_url`). Use `FacetType::Skill` when storing. Toolkit and identifier together form the unique identity; field is the attribute name.
- **Connected identities loader/renderer** — `src/openhuman/composio/providers/profile.rs` contains `load_connected_identities()` (reads `skill:*` facets) and `render_connected_identities_section()` (formats markdown for prompt injection). Keep rendering logic there, not in prompt modules.
- **Prompt injection helper** — `render_connected_identities` is imported and called in `welcome/prompt.rs`, `orchestrator/prompt.rs`, and `integrations_agent/prompt.rs` to inject a "Connected accounts:" block. Add it to any new agent prompt that needs Composio context.

## Agent Timeout & Cancellation (Issue #715)

- **Frontend silence timer, not a wall-clock limit** — `armSilenceTimer` in `app/src/pages/Conversations.tsx` fires if 120s (fixed to 600s) pass with zero inference progress events. It re-arms on every `tool_call`, `tool_result`, `iteration_start`, etc., so long-running tool chains that keep emitting events are not cut off.
- **Rust-side HTTP timeout is separate** — `src/openhuman/providers/compatible.rs` sets a 120s `reqwest` client timeout on LLM calls. Not changed in #715; relevant if a single LLM round-trip itself stalls for >2 min.
- **Manual cancel path** — `chatCancel()` in `app/src/services/chatService.ts` → `openhuman.channel_web_cancel` RPC → `cancel_chat()` in `src/openhuman/channels/providers/web.rs`. Fully implemented; the silence timer is an automatic fallback.

## Webhook & Cron Triggers (Issue #726)

- **Webhook bus was hardcoded 410** — `src/openhuman/webhooks/bus.rs` `WebhookRequestSubscriber::handle()` returned 410 "skill runtime removed" for ALL incoming webhooks. Now routes to echo/agent/skill/404 based on `TunnelRegistration.target_kind`.
- **WebhookRouter access from bus.rs** — Router lives in `SocketManager::shared.webhook_router` (was `pub(super)`). Added `pub fn webhook_router(&self)` accessor on `SocketManager`; bus.rs reaches it via `global_socket_manager().webhook_router()`.
- **`TriggerSource` enum: three update points** — Adding new variants requires updating: (a) `slug()` match in `envelope.rs`, (b) exhaustive test match, (c) `handle_triage_evaluate` string match in `agent/schemas.rs` (uses `p.source.as_str()`, not the enum directly).
- **`CronJobTriggered/CronJobCompleted` were never published** — Defined in `events.rs` and used in tests but never emitted. Now published by `execute_and_persist_job()` in `scheduler.rs`. Adding fields to these variants requires updating ~5 construction sites: `cron/bus.rs`, `composio/bus.rs`, `tree_summarizer/bus.rs`, `channels/proactive.rs`, and `events.rs` tests.
- **Webhook ops were all stubs** — `list_registrations`, `list_logs`, `clear_logs`, `register_echo`, `unregister_echo` in `ops.rs` all returned empty. Now backed by the real router via a `get_router()` helper.
- **`GGML_NATIVE=OFF` for cargo check** — Sidestepping the whisper-rs macOS Tahoe build blocker for `cargo check`: `GGML_NATIVE=OFF cargo check --manifest-path Cargo.toml`. Allows compilation checks without the cmake failure.

## Cron Scheduler

- **Cron loop was never spawned** — `tokio::spawn(cron::scheduler::run(config))` was missing from `src/core/jsonrpc.rs`. Added after the update scheduler spawn, gated on `config.cron.enabled`. Without it, scheduled jobs never auto-fire at startup (issue #830).

## Build & Tooling Gotchas

- **`pnpm typecheck` script was renamed** — Check `app/package.json` for the current name; as of issue #830 work, use `pnpm workspace openhuman-app compile` for tsc checks.
- **PR #745 (command palette) merged without its deps** — `@radix-ui/react-dialog`, `cmdk`, and `@testing-library/user-event` are missing from `package.json`. Install them if tsc fails after syncing main.
- **Pre-push hooks fail on upstream lint warnings** — ESLint warns on `setState` in effects and unused `eslint-disable` directives inherited from upstream. Use `--no-verify` only when the lint errors are pre-existing upstream issues, not new code.

## Environment

- **Core sidecar port** — `7788` (default). Check with `lsof -i :7788`.
- **Stage sidecar** — `cd app && pnpm core:stage` (required for core RPC).
- **Kill stuck processes** — `lsof -i :7788` then `kill <PID>`.
