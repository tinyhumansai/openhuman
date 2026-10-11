---
description: How OpenHuman tests its product with Vitest, cargo test and WDIO E2E, and where each kind of test goes.
icon: vial
---

# Testing strategy

This page answers "where does my test go?". It is the companion to [`TEST-COVERAGE-MATRIX.md`](https://github.com/tinyhumansai/openhuman/blob/main/docs/TEST-COVERAGE-MATRIX.md).

---

## Layers

| Layer                | Where it lives                                                                                                                                        | What it tests                                                                                                                                   | Driver                                                                                                        |
| -------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| Rust unit        | Sibling `<module>_tests.rs` files beside the module under `crates/openhuman-core/src/<domain>/`, or a `tests/` subdir under a domain (e.g. `crates/openhuman-core/src/channels/tests/`); inline `#[cfg(test)] mod` blocks and files named `tests.rs`/`test.rs` fail `pnpm rust:layout` | Pure domain logic, schemas, RPC handler shape, in-memory state machines                                                                         | `cargo test`                                                                                                  |
| Rust integration | `tests/*.rs` at repo root, each an explicit `[[test]]` target in `crates/openhuman-cli/Cargo.toml` (`autotests = false`); `tests/raw_coverage/*.rs` is globbed by `build.rs` into the single `raw_coverage_all` target | Full domain wiring with real Tokio runtime, mock external services, JSON-RPC end-to-end (`tests/json_rpc_e2e.rs`), domain × domain interactions | `pnpm test:rust` (which calls `bash scripts/test-rust-with-mock.sh`)                                          |
| Vitest unit      | Co-located as `*.test.ts(x)` next to source under `app/src/**`, or under `app/src/**/__tests__/`                                                      | React components, hooks, store slices, pure utilities, service-layer adapters                                                                   | `pnpm test` (root) or `pnpm --filter openhuman-app test:unit`                                                 |
| WDIO E2E         | `app/test/e2e/specs/*.spec.ts`                                                                                                                        | Full desktop flow: UI → Tauri → in-process core → JSON-RPC; user-visible behavior                                                              | All platforms: Linux CI drives the Wry-based debug build (macOS and Windows desktop E2E is disabled until a native driver exists). See [E2E testing](e2e-testing.md). |
| Manual smoke     | [`docs/RELEASE-MANUAL-SMOKE.md`](https://github.com/tinyhumansai/openhuman/blob/main/docs/RELEASE-MANUAL-SMOKE.md)                                                                                  | OS-level surfaces drivers cannot assert: TCC permission prompts, Gatekeeper, code signing, DMG install, OS-native toasts                        | Human at release-cut; the completed sign-off block is pasted as a GitHub commit comment on the `v<version>-staging` tagged commit QA validated. The promotion flow has no release PR. See [Release policy](release-policy.md) |

---

## Where does my test go?

```text
Is the change behind the JSON-RPC boundary (in `crates/openhuman-core/src/`, `crates/openhuman-rpc/`, or `crates/openhuman-embed/`)?
├─ YES - does it cross domains or talk to external services?
│   ├─ YES → Rust integration (tests/*.rs)
│   └─ NO  → Rust unit (next to source)
└─ NO - change is in `app/`
    ├─ Is it a pure function, hook, slice, or component in isolation?
    │   └─ YES → Vitest unit (*.test.tsx co-located)
    └─ Is it user-visible AND it crosses UI ⇄ Tauri ⇄ embedded core ⇄ JSON-RPC?
        ├─ YES → WDIO E2E (app/test/e2e/specs/*.spec.ts)
        └─ Is it OS-level (TCC, Gatekeeper, install, OS toasts)?
            └─ YES → Manual smoke checklist
```

If a change touches more than one layer, write a test in each layer it touches. Do not substitute one for another.

---

## Failure-path requirement

Every feature in the coverage matrix needs at least one failure or edge assertion as well as the happy path. Examples:

- File-write tool: happy = wrote bytes; failure = path-restriction denial.
- OAuth flow: happy = token issued; edge = expired refresh token recovery.
- Memory store: happy = stored + recalled; edge = forget-then-recall returns nothing.

A spec that asserts only the happy path is incomplete.

---

## Mock policy

- Use no real network in unit, integration or E2E tests. Use the shared mock backend (`scripts/mock-api-core.mjs`, `scripts/mock-api-server.mjs`, `app/test/e2e/mock-server.ts`).
- Admin endpoints for tests: `GET /__admin/health`, `POST /__admin/reset`, `POST /__admin/behavior`, `GET /__admin/requests`.
- External services (Telegram, Slack, Gmail, Notion, Ollama, OpenAI and others) are stubbed at the mock backend level; tests assert the request shape via `getRequestLog()`.
- The only acceptable exception is a documented release-cut manual smoke step.

---

## Determinism rules

- No wall-clock waits. Use the `waitForApp`, `waitForAppReady` and `waitForWebView` helpers, or explicit element-readiness checks.
- No shared filesystem state. Every E2E spec runs in an isolated `OPENHUMAN_WORKSPACE`, created and cleaned by `app/scripts/e2e-run-spec.sh`.
- No order-dependent specs. Each spec must pass when run alone.
- No reliance on absolute coordinates or animation timing.
- Prefer synthesizing keyboard input with `browser.execute(...)` over `browser.keys()`. See `command-palette.spec.ts` for the pattern.

---

## What the existing harness gives you

- Mock backend: `startMockServer` and `stopMockServer` in `app/test/e2e/mock-server.ts`.
- Auth shortcut: `triggerAuthDeepLink` and `triggerAuthDeepLinkBypass` in `helpers/deep-link-helpers.ts` skip real OAuth.
- Element helpers: `clickNativeButton`, `waitForWebView` and `clickToggle` in `helpers/element-helpers.ts`. Use these instead of raw `XCUIElementType*` selectors.
- Shared flows: `completeOnboardingIfVisible`, `navigateViaHash`, `navigateToSkills` and `walkOnboarding` in `helpers/shared-flows.ts`.
- Core RPC from a spec: `callOpenhumanRpc` in `helpers/core-rpc.ts` drives the embedded core directly when a UI step would be brittle.
- Platform guards: `isTauriDriver` and `supportsExecuteScript` in `helpers/platform.ts`. Desktop E2E runs the debug binary under Xvfb through `tauri-driver` on Linux. macOS and Windows have no automated desktop session yet.
- Artifact capture on failure: `captureFailureArtifacts` runs from `wdio.conf.ts`, and screenshots and DOM dumps land under `app/test/e2e/artifacts/`.

---

## Naming and structure

- WDIO specs: `<feature-area>-flow.spec.ts` for end-to-end product flows; `<feature>.spec.ts` for narrower surfaces.
- Vitest co-location: prefer `Component.tsx` + `Component.test.tsx` siblings; use `__tests__/` only when grouping multiple related tests.
- Rust integration tests: snake_case file matching the surface, `<feature>_e2e.rs` for JSON-RPC-driven flows, `<feature>_integration.rs` for cross-domain.
- Each `describe` block maps to a feature-list ID range. Link the matrix row in a comment if the mapping is not obvious.

---

## Pre-merge gates

Run these before opening a PR. CI runs the same set, but local runs are faster:

```bash
# Rust core
cargo fmt --check
cargo check --manifest-path Cargo.toml   # covers openhuman-core, openhuman-embed, openhuman-rpc, openhuman-tui
cargo clippy --manifest-path Cargo.toml -- -D warnings
cargo test --manifest-path Cargo.toml

# Tauri shell
cargo check --manifest-path crates/openhuman-app/Cargo.toml

# Frontend
pnpm typecheck
pnpm lint
pnpm format:check
pnpm test            # vitest, via the app workspace

# Rust integration with mock backend
pnpm test:rust

# E2E (slow, run when behavior changes visibly to users)
pnpm --filter openhuman-app test:e2e:build
bash app/scripts/e2e-run-spec.sh test/e2e/specs/<your-spec>.spec.ts <id>
```

---

## Manual smoke checks

WDIO and Appium cannot drive some surfaces, because they cross OS-level trust boundaries or hardware paths. [`docs/RELEASE-MANUAL-SMOKE.md`](https://github.com/tinyhumansai/openhuman/blob/main/docs/RELEASE-MANUAL-SMOKE.md) holds the full checklist and sign-off block. It is the source of truth for what to verify per release. It covers items such as:

- macOS TCC permission prompts (Accessibility, Input Monitoring, Microphone)
- Gatekeeper signature validation on first launch
- Code-sign integrity (`codesign --verify --deep --strict`)
- DMG install and drag-to-Applications flow
- Auto-update download and relaunch
- OS-native notification toasts on Linux (the driver sees no display server beyond Xvfb)

If a feature has no automated coverage and is not on the manual smoke list, treat it as untested and open a coverage gap.

---

## Coverage matrix as the contract

Every feature leaf in the [coverage matrix](https://github.com/tinyhumansai/openhuman/blob/main/docs/TEST-COVERAGE-MATRIX.md) maps to:

1. One or more test paths, or
2. A justified `🚫` with a manual-smoke entry.

When you add, remove or rename a feature, update its matrix row in the same PR.

---

## A red Rust suite locally is usually not a defect

Most local Rust failures here come from the environment, and they look exactly like product bugs. Work down this list before you conclude anything. Each step is one command. Each carries the failure you see without it, so you can check in thirty seconds whether it still holds.

Do not trust a note that tells you the cause. A line saying "these fail locally, it is provisioning" ends the investigation before it starts. Once it sent a worker hunting for a provisioning gap when the real cause was a product defect (`agent definition \`harness\` was not found`). Use the steps, not folklore.

### 1. Did anything compile?

`cargo` exits 101 in at least three cases that look identical from the shell: nothing compiled, a test asserted false, or the process aborted.

```bash
grep -c "^test result" run.log     # 0 = nothing ran
```

Zero `test result` lines means the problem is compilation or an abort, not a failing test. Read the first error line:

- `failed to load source for dependency <x>` or `No such file or directory` means a submodule is not checked out.
- `fatal runtime error: stack overflow` with `signal: 6, SIGABRT` and no `test result` line means the process aborted. See step 3.

### 2. Are the submodules there, at every depth?

Some crates depend on submodules nested inside other submodules. `openhuman-tinyhumans` is the clearest case. It needs `vendor/tinyhumans-sdk` (top level) and `vendor/tinyagents/vendor/tinytools` (nested). A plain `git submodule update --init` fetches the first and silently skips the second, so you get a compile failure that reads like a test failure.

```bash
git submodule update --init
git -C vendor/tinyagents submodule update --init --recursive
```

Without the nested module, `cargo test -p openhuman-tinyhumans` fails with no `test result` line:

```text
error: failed to get `tinytools` as a dependency of package `openhuman-cli v0.63.31`
  failed to load source for dependency `tinytools`
  No such file or directory (os error 2)
```

Check content, not status. `git submodule status` prints `-` only for unregistered modules, and registration happens before checkout. A half-finished init can therefore look ready while every working tree is empty. Seen mid-init: 13 `+`, 3 blank, zero `-`, while `du -sh vendor` said 120K and `ls vendor/tinychannels` was empty.

```bash
du -sh vendor                      # 120K = empty; a real checkout is GBs
ls vendor/<module> | wc -l         # 0 = empty, whatever status says
```

### 3. Is your feature set the one CI uses?

`cargo test -p openhuman --lib` on default features builds a smaller product than CI does, and the difference changes test outcomes, not just which tests exist. A feature flag can gate a tool's registration. A test can compare a static allowlist with what it finds by inspecting the built binary. The flag then moves the result silently, and the failure names a product concept, such as "agents that carry tools but whose prompt names none of them", with no mention of a feature.

In one comparison, `openhuman --lib` failed 5 tests on default features and 3 under CI's set. The two that differed were profile artifacts: `every_prompt_names_at_least_one_tool_it_can_call` depends on its default registry, and `the_withheld_block_renders_for_a_renamed_session_with_a_filter` needs `documents`. Reproduce CI exactly before you assert anything:

```bash
RUST_MIN_STACK=67108864 cargo test -p openhuman --lib \
  --features "$(bash scripts/ci/product-features.sh)"
```

`RUST_MIN_STACK` is per suite, not universal. Removing it from `openhuman-tinyhumans` still gives `183 passed; 0 failed`, so that crate does not need it. For `openhuman --lib`, `web_chat` is reported to abort with `fatal runtime error: stack overflow` without it, which is why the CI lane exports it. That abort has not been reproduced locally, so re-verify before relying on it elsewhere.

Two crates, one variable, opposite answers. Do not add a prerequisite by reflex. An unneeded one teaches contributors that the docs are unreliable.

### 4. Only now suspect a defect, and make the silence speak

If it compiled, the submodules are present and the feature set matches CI, a failure may be real. The trap at this stage is a layer that correctly discards detail, such as a sanitizer, scrubber or redactor. The symptom is a generic message where a specific one was produced.

Patch the discarding line to print what it drops, run once, and read the underlying error. In one case a sanitized `hosted agent invocation was rejected by policy` hid `Validation("agent definition \`harness\` was not found")`, which named the defect outright.

### Why this order

The cheap checks come first because a wrong answer early is invisible later. A suite that never compiled reports the same exit code as one that failed, and a plausible but wrong explanation gets confirmed instead of tested. Running CI's literal command is the surest way to avoid that.

---

## When in doubt

- Put the test as low in the stack as you can (Rust unit, then Rust integration, then Vitest, then WDIO). Lower layers are faster, more deterministic and cheaper.
- Use WDIO only for behavior that really crosses UI, Tauri, the embedded core and JSON-RPC. Do not drive a unit-testable concern through WDIO just because the UI exists.
- A failing happy path is a regression. A missing failure-path test is a gap. Both are bugs.
