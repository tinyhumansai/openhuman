# Plan: Remove all shipped screen-awareness and screen-capture capability

## Goal and invariants

Remove the shipped screen-awareness surface rather than disabling it. After this
work, removed JSON-RPC methods are unknown methods, removed CLI/Tauri/browser
actions are not advertised or dispatchable, and embedded CEF webviews cannot
obtain desktop-audio or desktop-video capture permission. Keep DOM/accessibility
text snapshots, user-provided images, normal vision processing of those images,
camera/microphone access, and test-runner/App Store artifact screenshots.

Work on the existing `remove-screen-awareness-fully` worktree. Do not alter the
committed approved design spec. Run the specified test before the deletion to
make it fail for the old surface where applicable, then make it pass in the
same atomic commit. Use `atomic-commit` with every listed path after validation.

## 1. Remove the core domain, config, accessibility capture, and public core contract

Files:

- Delete `src/openhuman/screen_intelligence/` in full, including its CLI,
  controller, engine, worker, vision, tool, and unit-test files.
- Delete `src/openhuman/accessibility/capture.rs`; remove the screen-capture
  exports from `src/openhuman/accessibility/mod.rs`.
- In `src/openhuman/accessibility/{permissions.rs,permissions_tests.rs,types.rs,README.md}`
  remove Screen Recording permission support, the `screen_recording` status
  field, and their tests; retain focused-text, foreground-window, automation,
  Globe, input-monitoring, and microphone behavior for now. The companion still
  consumes the foreground-window types until step 5.
- Remove the module/registry/CLI/legacy-alias entries from
  `src/openhuman/{mod.rs}`, `src/core/{all.rs,cli.rs,legacy_aliases.rs}`, and
  delete the `screen_intelligence` namespace description in `all.rs`.
- Remove `ScreenIntelligenceConfig` and its schema module/re-export/default
  field from `src/openhuman/config/{mod.rs,schema/accessibility.rs,schema/mod.rs,schema/types.rs}`.
  Remove its patch/update/controller/schema entries from
  `src/openhuman/config/{ops/mod.rs,ops/ui.rs,ops_tests.rs,schemas/helpers.rs,schemas/controllers.rs,schemas/schema_defs.rs,schemas_tests.rs,README.md}`.
  Do not add a TOML migration: deserializing a persisted unknown
  `[screen_intelligence]` table must remain accepted by serde and no longer
  produce a live config field.
- Remove the runtime snapshot type/build/degraded data and tests from
  `src/openhuman/app_state/{ops.rs,ops_tests.rs,README.md}` and the login/start/
  stop hooks/tests/docs in `src/openhuman/security/credentials/{ops.rs,ops_tests.rs,README.md}`.
- Remove the screen-derived app-state/config RPC cases and replace the current
  positive integration assertions in
  `tests/{config_auth_app_state_connectivity_e2e.rs,json_rpc_e2e.rs}` with one
  focused regression asserting `openhuman.screen_intelligence_status`,
  `openhuman.screen_intelligence_capture_now`, and
  `openhuman.config_update_screen_intelligence_settings` return JSON-RPC
  method-not-found. Delete `tests/screen_intelligence_vision_e2e.rs` and remove
  its suite entry from `scripts/test-rust-e2e.sh`.
- Remove only screen-intelligence assertions/fixtures from
  `tests/composio_list_tools_stack_overflow_regression.rs`,
  `tests/raw_coverage/tools_agent_credentials_state_raw_coverage_e2e.rs`, and
  `src/openhuman/tools/{ops_tests.rs,user_filter.rs}`. Preserve unrelated
  data-URL and user-image coverage.
- Remove only the `screenshot-ref` local CLI wrapper/list entry/tests from
  `src/openhuman/tools/local_cli.rs`, because it calls the deleted
  `screen_intelligence` module. Preserve the standalone `ScreenshotTool` wrapper
  for step 3.

Tests and checks:

1. First add the method-not-found and unknown-TOML-table assertions; confirm
   they fail against the current registered surface.
2. Run `cargo fmt --check`.
3. Run these focused library test commands separately:
   - `GGML_NATIVE=OFF cargo test --lib core::all::tests`
   - `GGML_NATIVE=OFF cargo test --lib config::schemas_tests`
   - `GGML_NATIVE=OFF cargo test --lib config::ops_tests`
   - `GGML_NATIVE=OFF cargo test --lib app_state::ops_tests`
   - `GGML_NATIVE=OFF cargo test --lib credentials::ops_tests`
4. Run `bash scripts/test-rust-with-mock.sh --test json_rpc_e2e` and
   `bash scripts/test-rust-with-mock.sh --test config_auth_app_state_connectivity_e2e`.
5. Run `GGML_NATIVE=OFF cargo check --manifest-path Cargo.toml`.

Commit:

`refactor(core): remove screen intelligence surface`

## 2. Remove screen-awareness agent discovery and capability catalog entries

Files:

- Delete `src/openhuman/agent/registry/agents/screen_awareness_agent/` and
  remove its module and `BuiltinAgent` entry from
  `src/openhuman/agent/registry/agents/{mod.rs,loader.rs}`.
- Remove it from the orchestrator's allowed subagents in
  `src/openhuman/agent/registry/agents/orchestrator/agent.toml`, from the
  built-in-definition assertion in
  `src/openhuman/agent/harness/builtin_definitions.rs`, and from expected agent
  counts/worker lists in `src/openhuman/agent/harness/definition_tests.rs` and
  `src/openhuman/agent/registry/agents/loader.rs` tests.
- Keep `vision_agent` but remove its two `screen_intelligence_*` tools and
  revise `src/openhuman/agent/registry/agents/vision_agent/{agent.toml,prompt.md}`
  to refer only to attached or on-disk user-provided images.
- Delete the screen-awareness prompt resource from
  `src/openhuman/mcp/server/resources.rs`.
- Delete `CapabilityCategory::ScreenIntelligence`, parsing/serialization tests,
  and all `screen_intelligence.*` catalog entries from
  `src/openhuman/platform/about_app/{types.rs,catalog_data.rs,catalog_tests.rs,README.md}`.
  Also remove the stale `screen_intelligence` example in
  `src/openhuman/overlay/types.rs` and screen-only mentions in
  `src/openhuman/inference/README.md`.

Tests and checks:

1. Update loader/catalog tests first so they assert the removed agent and
   capability category are absent while `vision_agent` still resolves with the
   image-capable hint; confirm they fail before removal.
2. Run `cargo fmt --check`.
3. Run these focused library test commands separately:
   - `GGML_NATIVE=OFF cargo test --lib agent_registry::agents::loader`
   - `GGML_NATIVE=OFF cargo test --lib agent::harness::definition_tests`
   - `GGML_NATIVE=OFF cargo test --lib about_app::`
4. Run `GGML_NATIVE=OFF cargo check --manifest-path Cargo.toml`.

Commit:

`refactor(agents): remove screen awareness discovery`

## 3. Remove native screenshot and pixel-returning browser actions

Files:

- Delete the standalone native `ScreenshotTool` implementation and tests in
  `src/openhuman/tools/impl/browser/screenshot.rs`; remove its module/re-export
  from `src/openhuman/tools/impl/browser/mod.rs`, its registration from
  `src/openhuman/tools/ops.rs`, and the remaining standalone screenshot
  wrapper/tests in `src/openhuman/tools/local_cli.rs`.
- In `src/openhuman/tools/impl/browser/{types.rs,action_parser.rs,browser.rs,browser_tests.rs}` remove the `BrowserAction::Screenshot` variant, parser,
  advertised action/schema option, and `screen_capture` computer-use action.
  Keep `snapshot` and all non-pixel browser/computer input actions.
- Remove screenshot execution branches from
  `src/openhuman/tools/impl/browser/{native_backend.rs,playwright_backend.rs,playwright_runner.mjs}`. Ensure every backend rejects `screenshot` and
  `screen_capture` as unsupported before sidecar dispatch.
- Remove browser screenshot expectations from
  `tests/raw_coverage/{tools_agent_credentials_state_raw_coverage_e2e.rs,tools_approval_channels_raw_coverage_e2e.rs,tools_channels_raw_coverage_e2e.rs}`
  and retain only assertions for the remaining browser API/data-url helpers
  that do not capture pixels.

Tests and checks:

1. Convert parser/backend tests to assert `screenshot` and `screen_capture`
   are unsupported and that `snapshot` still parses; these assertions must fail
   before the removal.
2. Run `cargo fmt --check`.
3. Run `GGML_NATIVE=OFF cargo test --lib
   openhuman::tools::implementations::browser::browser::tests` and
   `GGML_NATIVE=OFF cargo test --lib tools::ops_tests` separately.
4. Run `GGML_NATIVE=OFF cargo test --test tools_approval_channels_raw_coverage_e2e --test tools_channels_raw_coverage_e2e`.
5. Run `GGML_NATIVE=OFF cargo check --manifest-path Cargo.toml`.

Commit:

`refactor(tools): remove screenshot capture actions`

## 4. Remove Tauri display sharing and enforce CEF desktop-capture denial

Files:

- Delete `app/src-tauri/src/screen_capture/` and remove its module, managed
  `ScreenShareState`, and three command registrations from
  `app/src-tauri/src/lib.rs`.
- Remove `screen_share_begin_session`, `screen_share_thumbnail`, and
  `screen_share_finalize_session` and their screen-share descriptions from
  `app/src-tauri/permissions/{allow-webview-recipe.toml,allow-core-process.toml}`.
- Delete the complete `installGetDisplayMediaShim` block, picker DOM, session
  IPC calls, and display-capture permission-query override from
  `app/src-tauri/src/webview_accounts/runtime.js`; leave the ordinary recipe
  runtime and other webview functionality intact.
- In `app/src-tauri/vendor/tauri-cef/crates/tauri-runtime-cef/src/permissions.rs`,
  make `ALLOWED_MEDIA_MASK` contain only device microphone/camera bits and
  replace the desktop-allowed tests with tests that desktop audio, desktop
  video, and mixed device+desktop requests are filtered/denied while device
  audio/video remain allowed. Update the permission-handler comment in
  `cef_impl.rs` to state desktop bits are rejected.
- Remove obsolete native-picker claims from
  `app/src-tauri/src/{cdp/session.rs,meet_audio/captions_bridge.js}` without
  changing their retained mic/camera/caption behavior.

Tests and checks:

1. Change the CEF unit tests first to require desktop bits to be absent; verify
   the current mask makes them fail.
2. Run `cargo fmt --check` in `app/src-tauri`.
3. Run `cargo test --manifest-path app/src-tauri/vendor/tauri-cef/crates/tauri-runtime-cef/Cargo.toml permissions::tests`.
4. Run `cargo test --manifest-path app/src-tauri/Cargo.toml` and
   `cargo check --manifest-path app/src-tauri/Cargo.toml`.
5. Use `rg -n 'screen_share_|getDisplayMedia|DESKTOP_(AUDIO|VIDEO)_CAPTURE' app/src-tauri/src app/src-tauri/permissions` and require no remaining shipped
   command, shim, or allowlist match; the CEF denial implementation is checked
   separately in the vendored `permissions.rs` test above.

Commit:

`refactor(tauri): remove display sharing`

## 5. Make the desktop companion screen-independent

Files:

- Delete `app/src-tauri/src/companion/{pointing.rs,pointing_tests.rs}` and
  remove the module export from `companion/mod.rs`.
- In `app/src-tauri/src/companion/{mod.rs,pipeline.rs,types.rs,session.rs,session_tests.rs,pipeline_tests.rs}`, remove monitor geometry collection, foreground
  app/window context collection, screen-context prompt argument and text,
  `[POINT:…]` parsing, target fields, pointing state/transitions, and
  `capture_screen`/`include_app_context` config fields. `run_text_turn` and
  `run_audio_turn` should accept only the utterance/audio plus cancellation,
  send conversation history and the current utterance to the LLM, and return
  text/TTS/cancellation results without targets.
- After removing the last companion consumer, delete `AppContext` and the
  frontmost-window/`foreground_context` helpers and tests from
  `src/openhuman/accessibility/{types.rs,focus.rs,mod.rs,README.md}`. Preserve
  independently used focused-text helpers in the same files.
- In `app/src/store/{companionSlice.ts,companionSlice.test.ts}`, remove the
  `pointing` state and the two removed configuration fields.
- Remove the pointing-only frontend event/pointer surface from
  `app/src/services/{companionEvents.ts,__tests__/companionPayload.test.ts}` and
  `app/src/overlay/{CompanionPointer.tsx,OverlayApp.tsx,__tests__/CompanionPointer.test.tsx,__tests__/companionStateLabel.test.ts}`.
- In `app/src/components/settings/panels/{CompanionPanel.tsx,__tests__/CompanionPanel.test.tsx}`, remove the two screen-derived status rows and replace
  the old disabled-state test with coverage for the retained hotkey, activation
  mode, TTL, and session controls.

Tests and checks:

1. Update pipeline/session tests first to assert a text turn has no screen
   parameters, no pointing state, and a prompt containing only companion role,
   history, and utterance; verify the old signatures/state make this fail.
2. Run `cargo fmt --check` in `app/src-tauri`.
3. Run `cargo test --manifest-path app/src-tauri/Cargo.toml companion::` and
   `cargo check --manifest-path app/src-tauri/Cargo.toml`.
4. Run `cargo fmt --check`, `GGML_NATIVE=OFF cargo test --lib
   accessibility::`, and `GGML_NATIVE=OFF cargo check --manifest-path
   Cargo.toml` from the repository root for the shared accessibility cleanup.
5. Run `pnpm debug unit src/components/settings/panels/__tests__/CompanionPanel.test.tsx src/store/companionSlice.test.ts`.

Commit:

`refactor(companion): remove screen context and pointing`

## 6. Remove frontend screen-awareness UI, routes, core-state data, and RPC wrappers

Files:

- Delete `app/src/features/screen-intelligence/`,
  `app/src/components/intelligence/{ScreenIntelligenceDebugPanel.tsx,__tests__/ScreenIntelligenceDebugPanel.test.tsx}`,
  `app/src/components/settings/panels/{ScreenIntelligencePanel.tsx,ScreenAwarenessDebugPanel.tsx,screen-intelligence/,__tests__/ScreenIntelligencePanel.test.tsx,__tests__/ScreenAwarenessDebugPanel.test.tsx}`,
  `app/src/components/skills/{ScreenIntelligenceSetupModal.tsx,__tests__/ScreenIntelligenceSetupModal.test.tsx}`, and
  `app/src/utils/tauriCommands/accessibility.ts`.
- Remove all imports, cards, modal state, tab parsing, panel rendering, and
  skill icon from `app/src/{pages/Skills.tsx,components/intelligence/WorkflowsTab.tsx,components/skills/skillIcons.tsx,test/mockDefaultSkillStatusHooks.ts}`.
- Remove the screen-awareness settings route/registry/navigation/developer-row
  entries from `app/src/components/settings/{settingsRouteElements.tsx,settingsRouteRegistry.ts,hooks/useSettingsNavigation.ts,layout/settingsNavIcons.tsx,panels/DeveloperOptionsPanel.tsx}` and their affected tests. Do not add a
  redirect: old hashes must use the normal settings-router fallback.
- Remove `screenIntelligence` from the frontend snapshot types/store/provider in
  `app/src/{services/coreStateApi.ts,services/coreStateApi.test.ts,lib/coreState/store.ts,lib/coreState/__tests__/store.test.ts,providers/CoreStateProvider.tsx,providers/__tests__/CoreStateProvider.test.tsx,providers/__tests__/CoreStateProvider.identityFlip.test.tsx}` and fix each
  remaining snapshot fixture in component/oauth/store tests.
- Remove screen methods, aliases, accessibility-prefix remapping, schema-source
  dependency, and tests from `app/src/services/{rpcMethods.ts,__tests__/rpcMethods.test.ts,__tests__/coreRpcClient.test.ts}`. Remove the
  `screen_intelligence` capability category from
  `app/src/utils/tauriCommands/aboutApp.ts`.
- Delete all screen-awareness i18n keys, companion screen rows, and companion
  pointing label from every locale file:
  `app/src/lib/i18n/{ar,bn,de,en,es,fr,hi,id,it,ko,pl,pt,ru,zh-CN}.ts`.

Tests and checks:

1. Replace settings E2E expectations with assertions that
   `/settings/screen-intelligence` and `/settings/screen-awareness-debug` do
   not render a screen-awareness panel and follow existing settings fallback.
   Then delete the dedicated screen-intelligence specs:
   `app/test/{e2e,playwright}/specs/screen-intelligence.spec.ts`; revise
   `app/test/{e2e,playwright}/specs/settings-feature-preferences.spec.ts` and
   `app/test/playwright/specs/mcp-tab-flow.spec.ts` fixtures.
   Remove the deleted WDIO spec from the suite inventory in
   `app/scripts/e2e-run-all-flows.sh`.
2. Run `pnpm debug unit app/src/services/__tests__/rpcMethods.test.ts app/src/lib/coreState/__tests__/store.test.ts app/src/providers/__tests__/CoreStateProvider.test.tsx app/src/components/settings/panels/__tests__/DeveloperOptionsPanel.test.tsx`.
3. Run `pnpm i18n:check`, `pnpm i18n:english:check`, `pnpm typecheck`,
   `pnpm lint`, and `pnpm format:check`.

Commit:

`refactor(app): remove screen awareness UI`

## 7. Remove checked-in schema entries, product docs, and test inventory references

Files:

- Remove the `config_update_screen_intelligence_settings` and all
  `screen_intelligence_*` methods from the checked-in `app/schema.json` so it
  matches the registry removed in step 1.
- Delete `gitbooks/features/screen-intelligence.md` and remove screen/browser
  capture product claims from
  `gitbooks/features/native-tools/{README.md,browser-and-computer.md,image-tools.md}`.
  Preserve documentation of user-supplied image analysis and DOM snapshots.
- Remove the screen-capture rows from `docs/TEST-COVERAGE-MATRIX.md`, the
  obsolete desktop-automation wording in `docs/library-minimal-recipe.md`, and
  the relevant screen-intelligence claims in
  `docs/{RELEASE-MANUAL-SMOKE.md,tinyagents-inference-migration-plan.md}` and
  `gitbooks/developing/architecture/tauri-shell.md`.
- Remove `screen_capture` from the Tauri module inventory and any other stale
  screen-awareness architecture guidance in `AGENTS.md`.
- Revise `app/test/e2e/specs/tool-browser-flow.spec.ts` so it documents and
  tests `snapshot` rather than screenshot action availability. Do not change
  `app/test/e2e/helpers/artifacts.ts`, `app/test/e2e/specs/tauri-commands.spec.ts`,
  `gitbooks/developing/{agent-observability.md,e2e-testing.md,testing-strategy.md}`,
  or `scripts/ios-appstore-{assets,metadata}.mjs`: their screenshot references
  are explicitly non-shipped test/App Store artifacts.

Tests and checks:

1. Run `pnpm test:inventory` and, if the inventory generator changes a tracked
   output, include that generated result in this commit.
2. Run `pnpm docs:generate` followed by `pnpm docs:check`.
3. Run `pnpm debug e2e test/e2e/specs/tool-browser-flow.spec.ts` when the
   desktop test prerequisites are available; otherwise record that this is a
   CI validation and run the unit/type checks from steps 3 and 6 locally.
4. Run `git diff --check`.

Commit:

`docs: remove screen capture documentation`

## 8. Final whole-product removal audit

Run the following after all commits. If a shipped-surface match remains, fix it
in a new narrowly scoped atomic commit; do not rewrite already validated
history:

```bash
rg -n -i 'screen_intelligence|screen-awareness|screen awareness|screen_awareness_agent|screen_share_|getDisplayMedia|screen_capture' \
  src app/src app/src-tauri/src app/src-tauri/permissions docs gitbooks scripts tests app/schema.json \
  --glob '!docs/specs/2026-07-24-remove-screen-awareness-design.md' \
  --glob '!app/test/e2e/helpers/artifacts.ts' \
  --glob '!app/test/e2e/specs/tauri-commands.spec.ts' \
  --glob '!scripts/ios-appstore-assets.mjs' \
  --glob '!scripts/ios-appstore-metadata.mjs'
rg -n 'DESKTOP_(AUDIO|VIDEO)_CAPTURE' \
  app/src-tauri/vendor/tauri-cef/crates/tauri-runtime-cef/src/permissions.rs \
  app/src-tauri/vendor/tauri-cef/crates/tauri-runtime-cef/src/cef_impl.rs
```

The first command must have no shipped-product match. The second may reference
the CEF constants only in denial/filter tests or diagnostic formatting, never
in `ALLOWED_MEDIA_MASK` or an allow path. Manually confirm surviving words such
as “screenshot” refer only to user-provided images or the explicitly exempt
test/App Store artifacts.

Final validation:

```bash
cargo fmt --check
GGML_NATIVE=OFF cargo check --manifest-path Cargo.toml
cargo check --manifest-path app/src-tauri/Cargo.toml
pnpm typecheck
pnpm lint
pnpm format:check
pnpm i18n:check
pnpm i18n:english:check
pnpm docs:check
pnpm test:inventory
```

No additional commit is expected for this audit; any correction belongs to its
own earlier atomic boundary.
