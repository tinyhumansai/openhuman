# Project Memory

Quick reference for anyone starting with Claude on this project. Updated by the `memory-keeper` agent.

## Fixes & Gotchas

- **macOS close button does not dismiss window (issue #2049)** — `WebviewWindow::hide()` routes through CEF's `WindowMessage::Hide` → `cef::Window::hide()` which does NOT propagate to the visible NSWindow frame. Fix: use `AppHandle::hide()` which calls `[NSApp hide:]` via `set_application_visibility(false)`. This is macOS-only (`#[cfg(target_os = "macos")]`); the `CloseRequested` handler is in `app/src-tauri/src/lib.rs` around line 2809. PR #2118.
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
- **Always run checks before commit** — `pnpm workspace openhuman-app compile`, `pnpm lint`, `pnpm format:check`, `pnpm build`, `pnpm tauri dev`. Husky hooks enforce some but run all manually first.
- **Stage specific files** — Never `git add -A`. Always `git add <specific-files>`.

## Workflow

- **Agent order**: architectobot (plan) → user approval → codecrusher (implement) → architectobot (verify)
- **Always read CLAUDE.md first** before any issue work
- **Ask user when in doubt** — never assume scope or approach
- **PRs target upstream** — `tinyhumansai/openhuman` main branch, not fork
- **GraphQL project board can return empty** — `gh project item-list` on board #2 sometimes returns no items even when issues exist. Fall back to `gh issue list --repo tinyhumansai/openhuman` directly.
- **jq regex: use POSIX classes, not `\s`** — jq's `test()` uses ONIG regex; `\s` is not supported. Use `[[:space:]]` for whitespace matching in `gh pr list --json ... --jq` pipelines.
- **PR conflict check: `Closes #N` syntax not always used** — `gh pr list --jq "select(.body | test('Closes #N'))"` misses PRs that mention an issue thematically without a closing keyword. Also search PR title + body for the raw issue number (`#N`) with broader matching to catch related open PRs before claiming an issue is unassigned.
- **`pnpm debug unit` path is relative to `app/src/`** — Pass `providers/__tests__/Foo.test.tsx`, not `app/src/providers/__tests__/Foo.test.tsx`.
- **Prettier must run after codecrusher adds test cases** — New test blocks often fail `format:check`. Run `pnpm --filter openhuman-app format` before committing when test files are touched.
- **Check for existing PRs before implementing** — When the workflow picks an issue, search open PRs for the issue number and related keywords before starting work. A contributor may have already shipped the fix (e.g. PR #2101 for issue #2075).
- **Project board `gh project item-list` paginates closed items first** — The first 100 items returned are often CLOSED. Must `--limit 500` or paginate to find open/unassigned work. Fall back to `gh issue list --repo tinyhumansai/openhuman --state open` for reliability.

## Local AI Presets

- **Tier system lives in `src/openhuman/local_ai/presets.rs`** — single source of truth for tier→model ID mapping. To change default models for a release, edit `all_presets()` there.
- **Device detection** uses `sysinfo` crate (`src/openhuman/local_ai/device.rs`). Apple Silicon = GPU always; others = best-effort.
- **`OPENHUMAN_LOCAL_AI_TIER` env var** overrides the selected tier at config load time (in `load.rs`).
- **Frontend tier selector** is in `LocalModelPanel.tsx` under Settings > Local AI Model. Uses `coreRpcClient` to call 3 RPC methods: `local_ai_device_profile`, `local_ai_presets`, `local_ai_apply_preset`.
- **Default config maps to Medium tier** (`gemma3:4b-it-qat`). If someone changes `model_ids.rs` defaults, they should keep `presets.rs` in sync.
- **`ollama_base_url()` previously ignored `config.local_ai.base_url`** — It only read env vars. Fixed in feat/ollama-external-server-url by adding `ollama_base_url_from_config(config)`. Any new Ollama URL resolution must go through the config-aware helper, not the env-only one.
- **`LocalModelDebugPanel.tsx` must seed URL from config on mount** — Previously initialized `ollamaBaseUrlInput` to the hardcoded default and only loaded the persisted URL when diagnostics ran. Fix: `useEffect` on mount calls `openhumanGetConfig()` and sets state from `config.local_ai.base_url`. Pattern to follow for any settings field backed by Rust config.

## Core process (in-process, no sidecar)

- **Core runs in-process** as a tokio task inside the Tauri host (sidecar removed in PR #1061). Lifecycle owned by `core_process::CoreProcessHandle` in `app/src-tauri/src/core_process.rs`.
- **`pnpm core:stage` is a no-op echo** — there is no `target/debug/openhuman-core` binary to copy into `app/src-tauri/binaries/`. Rebuilding via `pnpm dev:app` is enough to pick up new RPC methods in the in-process server.
- **Token auth**: per-launch hex bearer in `OPENHUMAN_CORE_TOKEN`, exposed to the renderer via `core_rpc_token` Tauri command. Always call RPC through `invoke('core_rpc_relay', { request: { method, params } })` — avoids the CORS preflight `fetch()` would trigger.
- **Stale-listener policy** (#1130): if the port is already in use, the handle probes `GET /` to decide if it's an OpenHuman core, then term/force-kill with PID revalidation guarding against PID reuse. Set `OPENHUMAN_CORE_REUSE_EXISTING=1` to attach to a manually-started `openhuman-core serve` for debugging.
- **Default port** `7788`. Stage token (when running standalone): `~/.openhuman-staging/core.token` under `OPENHUMAN_APP_ENV=staging`.

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
- **Walkthrough is multi-page (9 steps)** — Uses react-joyride v3 `Step.before` async hooks to navigate between pages (`/home → /chat → /skills → /intelligence → /settings → /home`). Steps factory: `createWalkthroughSteps(navigate)` in `walkthroughSteps.ts`. `waitForTarget(selector, timeout)` polls via rAF until DOM target appears. Re-trigger from Settings via `resetWalkthrough()` + `walkthrough:restart` CustomEvent. `AppWalkthrough` is mounted inside Router context (can use `useNavigate` directly). BottomTabBar attr is `tab-notifications` (not `tab-automation`).
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
- **`bootstrapFailCountRef` retry counter bug (issue #2158)** — The ref is a cumulative lifetime counter; logging it against `MAX_BOOTSTRAP_RETRIES` (5) as denominator produced impossible `attempt 11/5`. Fix: distinguish bootstrap phase ("attempt X/5") from continuous-poll phase (separate message, 10s backoff). Reset the counter to 0 on any successful snapshot fetch.
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

## Composio Backend URL Bug (Issue #2075, PR #2101)

- **`effective_backend_api_url` env-fallback branch skipped normalization** — In `src/api/config.rs`, the override branch normalized via `normalize_backend_api_base_url` but the env-fallback branch (`OPENHUMAN_BACKEND_API_URL`) did not, so scheme-less URLs like `api.example.com` were used raw. Fix: normalize the env-fallback branch too (3-layer defense: config → env-fallback → `IntegrationClient::new`).
- **`normalize_backend_api_base_url` and `redact_url_for_log` are `pub(crate)`** — Available for reuse across `src/api/` after PR #2101 merge.

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

## Agent Runtime Behavior

- **`sandbox_mode = "read_only"` in agent.toml is metadata only** — Never enforced at runtime. Actual security policy comes from `config.autonomy` (global), defaulting to `Supervised`. Adding write tools to a read-only agent works at runtime but violates documented intent.
- **`max_iterations` hard-fails, not graceful truncation** — When the welcome agent (or any agent) hits `max_iterations`, `tool_loop.rs:705` calls `anyhow::bail!`. There is no graceful truncation. Budget iterations carefully.
- **Archivist agent auto-extracts memory** — It processes conversation history and persists preferences/facts into `user_profile` automatically. Agents do not need to explicitly call `memory_store` to persist conversational insights.
- **`cargo check` / `cargo test` fails on main (llama.cpp cmake)** — `llama.cpp`'s cmake build script uses `-mcpu=native`, which is unsupported on Apple clang 21+ with `--target=arm64-apple-macosx`. Pre-existing issue on `main`, not branch-specific. Frontend checks (typecheck, lint, format) are unaffected. Workaround: set `GGML_NATIVE=OFF` (same fix as whisper-rs above).

## Cron Scheduler

- **Cron loop was never spawned** — `tokio::spawn(cron::scheduler::run(config))` was missing from `src/core/jsonrpc.rs`. Added after the update scheduler spawn, gated on `config.cron.enabled`. Without it, scheduled jobs never auto-fire at startup (issue #830).

## Build & Tooling Gotchas

- **`pnpm typecheck` script was renamed** — Check `app/package.json` for the current name; as of issue #830 work, use `pnpm workspace openhuman-app compile` for tsc checks.
- **PR #745 (command palette) merged without its deps** — `@radix-ui/react-dialog`, `cmdk`, and `@testing-library/user-event` are missing from `package.json`. Install them if tsc fails after syncing main.
- **Pre-push hooks fail on upstream lint warnings** — ESLint warns on `setState` in effects and unused `eslint-disable` directives inherited from upstream. Use `--no-verify` only when the lint errors are pre-existing upstream issues, not new code.
- **`pnpm tauri icon <source.png>` generates all platform icons at once** — Produces `.icns`, `.ico`, all PNG sizes, Windows Store tiles, and iOS/Android sets. Use this instead of manual `sips`/ImageMagick resizing.
- **`tauri-cef` submodule update can fix missing Tauri runtime modules** — e.g. updating to f75bc21f5 added the missing `tauri_runtime_cef::audio` module that was causing pre-push hook compile failures on the Tauri shell. When the shell fails to compile with a missing module error, check if the submodule needs updating.
- **`git add` must run from repo root** — Staging paths like `app/public/...` with `git add` from inside `app/` won't match. Always run `git add` from `/Users/megamind/tinyhuman/openhuman-claude`.
- **Brand kit assets live at `app/public/brand/`** — Copied there during session work; original source is in `~/Downloads/Brand kit/`. Not auto-synced; re-copy manually if Downloads content changes.
- **`pnpm test:coverage` ENOENT on `coverage/.tmp/coverage-0.json`** — Race condition in coverage file collection; flaky, not reproducible every run. Use `pnpm debug unit` instead — runs Vitest without coverage, faster and reliable for iteration.

## Mascot Native Window (macOS)

- **Not a Tauri window** — The floating mascot is a native `NSPanel` + `WKWebView` in `app/src-tauri/src/mascot_native_window.rs`. It uses `ignoresMouseEvents=true` (click-through); interaction is detected by polling `NSEvent` via a Foundation timer. macOS-only, uses objc2 bindings.
- **`MainThreadOnly` import must stay** — Required by `WKWebView::alloc()` and other AppKit allocators even if not explicitly referenced in user code. Removing it causes compile errors.
- **`NSEvent::pressedMouseButtons` not in typed objc2-appkit bindings** — Must be called via `msg_send!(objc2::class!(NSEvent), pressedMouseButtons)` instead of the typed API.
- **WKWebView IPC via `evaluateJavaScript`** — The mascot webview is NOT a Tauri runtime; Tauri `invoke`/`emit` do NOT work. Rust-to-mascot communication uses `msg_send!(webview, evaluateJavaScript:completionHandler:)` to dispatch `new CustomEvent(...)` on `window`. React listens with `addEventListener`. This is NOT subject to the CEF JS injection ban (that only applies to `webview_accounts/` third-party origins).
- **`MascotCharacter` `sleeping` prop** — Drives the sleep animation (eye close + Zzz). `sleepStartSec` and `sleepFullSec` are hardcoded at 2.5s and 4.0s — they are NOT configurable props. Only toggle `sleeping: boolean`.
- **`FACE_PRESETS` is a strict `Record`** — Typed as `Record<Exclude<MascotFace, 'normal'>, FacePreset>` in `Ghosty.tsx`. Adding a new `MascotFace` union variant requires adding a matching entry to `FACE_PRESETS` or it won't compile.
- **`_webview` in `spawn_drag_timer`** — The `WKWebView` captured in the drag timer closure was originally unused (prefixed `_`). It can be used for `evaluateJavaScript` calls during the hover polling loop (e.g. to trigger blink/wake events from Rust).
- **FrameProvider loops — sleep animation resets** — `FrameProvider` uses `frame % durationInFrames` so animations loop. Default `DURATION_FRAMES = FPS * 6` (6s). Sleep animation completes at 4s, then eyes re-open at 6s when frame resets to 0. Fix: use a much longer `durationInFrames` for sleep face (e.g. `FPS * 600`) so the loop never triggers while sleeping.
- **Hover detection needs circular hitbox** — The mascot panel is 79x79 but the character is visually circular. Using the full AABB (`cursor_in_panel`) for hover triggers false positives when cursor is in a panel corner. Use distance-from-center check instead. Also suppress hover events for ~1s after panel shows to let the webview load.

## Google Analytics (Issue #1479)

- **`react-ga4` injects a `<script>` tag at runtime** — It appends a `gtag.js` `<script>` to `<head>` dynamically. This works because `tauri.conf.json` CSP has `https:` in `default-src` and `connect-src`. If CEF ever tightens `script-src` separately, switch to GA4 Measurement Protocol (pure HTTP POST, no script injection).
- **Analytics module pattern** — `app/src/services/analytics.ts` is the single owner of `initGA`, `trackPageView`, `trackEvent`, plus an `ALLOWED_EVENTS` allowlist. Never call `ReactGA` directly from components; go through this module.
- **Triple gate before any GA call** — `isAnalyticsEnabled()` (user consent) AND `GA_MEASUREMENT_ID` env var present AND `!IS_DEV`. All three must pass or tracking is silently skipped.
- **Route tracking location** — `useLocation()` effect wired in AppShell (not individual pages). All page views emit from one place.
- **Capability catalog must stay in sync** — `src/openhuman/about_app/catalog.rs` needs an entry when a new user-visible feature ships. GA was added there as part of issue #1479.

## PR Checklist CI

- **N/A items need a checked checkbox** — `scripts/check-pr-checklist.mjs` requires `- [x] N/A: <reason>`. Using `- [ ] N/A:` (unchecked) fails the check even though the text starts with "N/A:".

## Config System (Rust)

- **Config corruption recovery** — `parse_config_with_recovery` in `src/openhuman/config/schema/load.rs`: try primary → try `.bak` → archive corrupt file → `Config::default()`. Guarantees the app always starts even with a corrupt config.
- **New config fields must use `#[serde(default = "fn_name")]`** — Bare `#[serde(default)]` gives `0`/`false`, not the meaningful domain default. Define a named fn returning the correct value and reference it by name.
- **`.bak` is now permanent** — `Config::save()` no longer deletes `.bak` on success. It always reflects the last-known-good config before the most recent write.
- **`load_from_default_paths` has zero callers** — Debug utility only; not user-facing.
- **Config test module path** — `openhuman::config::schema::load::tests`. Run with `cargo test -- config::schema::load::tests`.

## Environment

- **Core port** — `7788` (default; in-process inside Tauri host). Check with `lsof -i :7788`.
- **`pnpm core:stage`** — no-op (sidecar removed in PR #1061). Use `pnpm dev:app` for full Tauri+core dev.
- **Kill stuck processes** — `lsof -i :7788` then `kill <PID>`. Useful when `dev:app` reports a stale listener and you want to force a fresh boot rather than relying on the handle's auto-recovery.
- **Skills runtime rebuilt (PR #2707)** — QuickJS is gone, but skills now run as orchestrator-focused agents via `skills_run` RPC. Default skills live in `src/openhuman/skills/defaults/<id>/` with `skill.toml` + `SKILL.md`, registered in `registry.rs` `DEFAULT_SKILLS` const. Seeded into `<workspace>/skills/` on boot (idempotent, non-destructive). Bundled defaults: `github-issue-crusher`, `dev-workflow`. Skills run with 200 iteration cap and full web access.
- **Codegraph tools (PR #2707)** — `codegraph_index` and `codegraph_search` registered in `src/openhuman/tools/ops.rs`. Implementation in `src/openhuman/codegraph/` — tree-sitter extraction, SQLite FTS5, dense embeddings, RRF fusion. Auto-indexes on first search.
- **Tool names are exact** — Always check `src/openhuman/tools/ops.rs` for authoritative names. Key ones: `edit` (not `edit_file`), `composio` (not `composio_execute`), `codegraph_index`, `codegraph_search`.
- **`cron_add` RPC** — Was missing from `schemas.rs` (only existed as agent tool). Now exposed as `openhuman.cron_add`. Frontend wrapper: `openhumanCronAdd()` in `app/src/utils/tauriCommands/cron.ts`.
- **Worktree `pnpm build` rolldown fix** — Worktrees can miss `@rolldown/binding-darwin-arm64`. Fix: `pnpm install --force`.

## Artifacts Domain (Issue #2776)

- **Filesystem-backed persistence, no SQLite** — `src/openhuman/artifacts/` stores JSON metadata (`meta.json`) + binary blobs under `<workspace_dir>/artifacts/<uuid>/`. Pattern mirrors `memory/ops/files.rs` but simpler.
- **`"ai"` namespace in controller registry** — RPC methods are `openhuman.ai_list_artifacts`, `openhuman.ai_get_artifact`, `openhuman.ai_delete_artifact`. Future `ai_*` methods should use this same namespace.
- **Two-layer path validation required** — (1) `validate_artifact_id` rejects empty strings, `/`, `\`, `..`, absolute Unix paths, Windows `C:` and UNC `\\` paths; (2) `assert_within_root` canonicalizes and checks containment. Replicate this pattern for any new filesystem-backed domain.
- **`cargo test --lib` required for lib crate tests** — `cargo test -p openhuman -- "artifacts"` lists tests but filters to 0. Must use `cargo test -p openhuman --lib -- "artifacts"` because tests are in the lib crate, not integration test binaries.

## Rust Testing Patterns

- **Memory tree tests filter** — `cargo test -p openhuman -- "memory::tree"` runs the memory tree unit tests (602 tests); full module paths are `openhuman::memory::tree::ingest::tests::*` and `openhuman::memory::tree::canonicalize::email_clean::tests::*`.
- **`cargo fmt --all`** — Required after codecrusher generates Rust; it doesn't always produce perfectly formatted output and CI will reject unformatted code.
- **PR quality scripts are soft checks** — `scripts/check-pr-checklist.mjs` and `scripts/check-coverage-matrix.mjs` exit cleanly with summary lines; CI treats them as advisory, not blocking.
- **`ceil_char_boundary`** — Safe string slicing utility at `src/openhuman/util.rs`; use this throughout the codebase instead of raw byte-index slicing to avoid UTF-8 panics.
- **Global static cache tests need a reset guard** — When testing code that reads/writes a `Lazy<Mutex<Option<...>>>` global cache, use a `struct CacheResetGuard; impl Drop for CacheResetGuard { fn drop(&mut self) { *CACHE.lock() = None; } }` pattern so each test starts clean. See `SnapshotCacheResetGuard` / `CacheResetGuard` in `ops_tests.rs`.
- **Test assertions must match the actual dummy value** — When a builder (e.g. `build_dummy_runtime_snapshot()`) wraps `degraded_runtime_snapshot()`, assert against `dummy.field` rather than a hardcoded string (e.g. `"idle"` vs the actual `"degraded"`) to verify round-trip correctness without false mismatches.
- **`composio::action_tool::tests::mode_toggle_between_calls_is_observed` is flaky in full suite** — Fails intermittently due to shared global composio session state; passes in isolation. Pre-existing; not caused by snapshot perf work.
- **`GLOBAL_MEMORY_TEST_LOCK` only serializes test bodies, not background workers** — Background ingestion spawned by a prior test can still be running when the next test acquires the lock. Call `state.reset_for_test()` at test start (after acquiring the lock) to clear accumulated `queue_depth`/`running` state; do not rely on delta assertions alone.
- **`IngestionState::reset_for_test()` is `#[cfg(test)]`-gated** — Lives in `src/openhuman/memory/ingestion/state.rs`. Zeroes `queue_depth` (AtomicUsize) and clears running/current fields in the snapshot while preserving completion history. This is the canonical reset for any test asserting exact queue or running state.
- **cargo-llvm-cov widens SQLITE_BUSY window** — Flakes that only appear under coverage (`cargo-llvm-cov`) but not plain `cargo test` are usually (a) a SQLite connection missing `busy_timeout`, or (b) shared global state not reset between tests. Always set `busy_timeout` on new SQLite connections (see pattern below).
- **All new SQLite connections must set `busy_timeout = 15s`** — Call `conn.busy_timeout(Duration::from_secs(15))` immediately after `Connection::open()`, before any `execute_batch()`. Pattern set by `chunks/store.rs` (`SQLITE_BUSY_TIMEOUT`) and now also used by `memory_store/unified/init.rs` (fixed in issue #2722). Without it, concurrent ingestion + test writes produce `SQLITE_BUSY` under cargo-llvm-cov.

## App State Snapshot (Issue #2155 — first-launch perf)

- **`build_runtime_snapshot` was serial, now parallel** — The four subsystems (screen intelligence, local AI, autocomplete, service status) in `src/openhuman/app_state/ops.rs` ran sequentially. Fixed with `tokio::join!`. Also added a 2s TTL cache (`RUNTIME_SNAPSHOT_CACHE`) so repeated polls within the TTL skip recomputation.
- **`service::status` is sync — must use `spawn_blocking`** — `crate::openhuman::service::status(config)` may shell out to `launchctl`. Wrap it in `tokio::task::spawn_blocking` when called from an async context.
- **`autocomplete::global_engine().status()` calls `Config::load_or_init()` internally** — Avoid this inside snapshot code. Use the new `status_with_config(config)` method which accepts an already-loaded config.
- **Per-stage snapshot timeouts** — `AUTH_FETCH_TIMEOUT = 5s` and `RUNTIME_SNAPSHOT_TIMEOUT = 10s` are constants in `ops.rs`; they sum to 15s, well under the 30s frontend RPC timeout.
## Project Board & Issue Queries

- **Project #2 paginates at 100 items** — Board has 627+ items. Use GraphQL cursor pagination to find all open P0 issues; a single query only returns the first 100.
- **jq regex `\s+` causes parse errors** — Use plain `test("#NNNN")` to check if a PR/issue body references an issue number. `\s+` in jq regex triggers parse errors.
- **Most open P0s are security or Linux AppImage GLIBC issues** — When triaging P0s, filter for those categories first.
- **Project #2 shows only closed items on the board view** — Use `gh issue list --repo tinyhumansai/openhuman --state open --assignee ""` to find unassigned open issues instead of querying the project board.
- **Check linked PRs via timeline API, not body regex** — `gh api repos/tinyhumansai/openhuman/issues/$N/timeline --paginate | jq '[.[] | select(.event == "cross-referenced" and .source.issue.state == "open")] | length'` is more reliable than searching issue body text for PR references.

## Git Submodules

- **`tauri-cef` and `tauri-plugin-notification` are git submodules** — When upstream/main updates them, fix with `git submodule update --remote --checkout`, not by manually patching the vendored crate.

## Pre-existing Test Failures

- **`composio::action_tool::tests::factory_routes_through_direct_when_mode_is_direct` fails in `cargo test -p openhuman`** — Pre-existing failure unrelated to WhatsApp or any recent branch work. Do not attempt to fix unless explicitly tasked. Also intermittently flaky when run as part of the full suite — see "Pre-existing Flaky Tests" section.

## Workflow Gate (must not skip)

- **Steps 4–6 of `workflow/00-full-workflow.md` are mandatory before committing** — Step 4: architectobot verify. Step 5: full checks (`pnpm test:coverage`, `pnpm build`, `bash scripts/install.sh --dry-run`, PR quality scripts). Step 6: memory-keeper. Skipping any of these violates the workflow contract.
- **Encode architectobot answers in the codecrusher prompt** — When the architectobot plan includes clarifying questions and the user approves specific answers, embed those decisions as explicit constraints in the codecrusher prompt so the agent doesn't re-ask.

## Security Policy

- **Path validation entry point** — `src/openhuman/security/policy.rs` exposes `validate_path` / `validate_parent_path`. All file I/O path validation must go through this API. `is_path_string_allowed()` is a string-only first pass, not sufficient on its own.
- **validate_parent_path before create_dir_all** — For write operations, `validate_parent_path` MUST be called before any `create_dir_all` call. Calling it after allows symlink attacks to create directories outside the workspace before the security check fires (Issue #1927).
- **Tool callers must use `validate_path` / `validate_parent_path`** — All tool implementations under `src/openhuman/tools/impl/filesystem/` must use these functions, not the legacy `is_path_allowed` / `is_resolved_path_allowed`.
- **Security policy test filter** — Run only security policy tests with: `cargo test -p openhuman -- "security::policy"`. Runs the 100 tests in `src/openhuman/security/policy_tests.rs` cleanly.

## Pre-existing Flaky Tests

- **`composio::action_tool` and `agent::harness::session::turn` intermittent failures** — These tests fail randomly when run as part of the full suite (likely shared state or timing), but pass individually. Not related to security/policy changes. Do not treat as blockers for security-module PRs.

## Windows OAuth Deep Link (Issue #2562)

- **Three-layer fix**: (1) named-pipe IPC in `deep_link_ipc_windows.rs` — secondary process forwards `openhuman://` URL to primary via `\\.\pipe\com.openhuman.app-deeplink`, 40 retries × 50ms; (2) loopback OAuth server in `loopback_oauth.rs` — RFC 8252 one-shot `127.0.0.1:53824`, preferred path that eliminates deep link dispatch entirely; (3) Linux analog in `deep_link_ipc.rs` — Unix domain socket at `$XDG_RUNTIME_DIR/com.openhuman.app-deeplink.sock`.
- **`OAuthProviderButton.tsx` loopback flow** — tries loopback first, sets `redirectUri` for backend, awaits callback, rewrites `http://127.0.0.1:PORT/auth?...` → `openhuman://auth?...` → `handleDeepLinkUrls`. Falls back to deep link if bind fails.
- **Pipe binding location** — primary binds the named pipe in `lib.rs` right after the mutex guard (line 2269); `drain_pending_urls()` wired in `setup()` at line 2578.
- **Issue was already fixed before we picked it up** — PRs #2469, #2511, #2550 had already merged the fix. Our contribution was extracting `classify_request` as a pure function and adding 11 Rust unit tests.
- **Pure-function extraction pattern** — when async/AppHandle-gated Tauri code is untestable, extract a `classify_request(head, expected_state, bound_port) -> RequestOutcome` pure function returning an enum. Enables comprehensive unit tests with zero Tauri context. `RequestOutcome` has 4 variants: `AuthCallback`, `StateMismatch`, `NotFound`, `MethodNotAllowed`.

## Port Conflict Recovery (Issue #2617)

- **Port fallback already in `pick_listen_port`** — `src/openhuman/connectivity/rpc.rs` tries ports 7789–7798 when 7788 is busy. Gap was: frontend `getCoreRpcUrl()` cached the URL on first resolution so it never picked up the fallback port, and stale-process reaping was macOS-only.
- **`process_recovery.rs` is platform-gated** — `reap_stale_openhuman_processes` had only a macOS impl. Linux uses `/proc/<pid>/cmdline`; Windows uses `wmic process get`. Tests for each platform's parsing logic live in the same file, following the existing macOS test pattern.
- **`recover_port_conflict` is a Tauri IPC command, not JSON-RPC** — Rust E2E test for port fallback lives in `tests/json_rpc_e2e.rs` and calls `pick_listen_port` directly: bind port 7788 with a `std::net::TcpListener` (std, not tokio) to simulate conflict, confirm fallback, then serve via `tokio::net::TcpListener::from_std(pick_result.listener.into_std())`.
- **`BootCheckTransport` is the right hook for frontend recovery** — `app/src/lib/bootCheck/index.ts` is the injection point for new recovery capabilities; don't add them directly to the BootCheck component.
- **i18n bootCheck keys live in `-3.ts` chunks** — New keys must be added to all 13 language files simultaneously (the `-3.ts` chunk for each language).
- **Workflow folder** — `workflow/` at repo root has 5 markdown files (00–05) defining the full PR workflow: pick issue → architectobot plan → user approval → codecrusher → architectobot verify → checks → memory-keeper → commit → push/PR.

## Channel Event Workspace Routing (Issue #2602)

- **Workspace identity is `PathBuf`** — Represented as the workspace directory path on `ChannelRuntimeContext` as `ctx.workspace_dir: Arc<PathBuf>`. Use `ctx.workspace_dir.as_ref().clone()` at publish sites. There is no abstract `WorkspaceId` type.
- **`DomainEvent` workspace routing contract** — Publisher populates workspace field from context; subscriber compares against `self.workspace_dir` and early-returns with `log::debug!` on mismatch. Follow this pattern for any workspace-scoped `DomainEvent` variant.
- **`ChannelMessageReceived` and `ChannelMessageProcessed` carry `workspace_dir`** — Added in PR for issue #2602. Guards in `ConversationPersistenceSubscriber` (memory_conversations/bus.rs) and `TelegramRemoteSubscriber` (telegram/bus.rs) prevent cross-workspace persistence during login/workspace-change races.

## Pre-existing Upstream Failures (from issue #2602 session)

- **Upstream `main` has 5 Vitest failures and 4 TypeScript compile errors** — Caused by missing iOS experimental dependencies: `@noble/ciphers/chacha`, `@noble/ciphers/webcrypto`, `qrcode.react`, `@tauri-apps/plugin-barcode-scanner`. Breaks `pnpm compile`, `pnpm build`, `pnpm test:coverage` on a clean checkout. Always verify by stashing changes and running checks on the base branch before blaming your PR.
- **`cargo fmt` must run after codecrusher** — codecrusher does not reliably produce `cargo fmt`-clean Rust. Always run `cargo fmt --manifest-path Cargo.toml` after codecrusher finishes and before committing.
