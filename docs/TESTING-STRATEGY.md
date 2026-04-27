# Testing Strategy

How OpenHuman tests its product. Source of truth for "where does my test go?". Companion to [`TEST-COVERAGE-MATRIX.md`](./TEST-COVERAGE-MATRIX.md).

---

## Layers

| Layer                | Where it lives                                                                                                                                        | What it tests                                                                                                                                   | Driver                                                                                                                     |
| -------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| **Rust unit**        | `#[cfg(test)] mod tests` inside the same `*.rs` file, or sibling `tests.rs`, or `tests/` subdir under a domain (e.g. `src/openhuman/channels/tests/`) | Pure domain logic, schemas, RPC handler shape, in-memory state machines                                                                         | `cargo test`                                                                                                               |
| **Rust integration** | `tests/*.rs` at repo root                                                                                                                             | Full domain wiring with real Tokio runtime, mock external services, JSON-RPC end-to-end (`tests/json_rpc_e2e.rs`), domain × domain interactions | `pnpm test:rust` (which calls `bash scripts/test-rust-with-mock.sh`)                                                       |
| **Vitest unit**      | Co-located as `*.test.ts(x)` next to source under `app/src/**`, or under `app/src/**/__tests__/`                                                      | React components, hooks, store slices, pure utilities, service-layer adapters                                                                   | `pnpm test:unit`                                                                                                           |
| **WDIO E2E**         | `app/test/e2e/specs/*.spec.ts`                                                                                                                        | Full desktop flow: UI → Tauri → core sidecar → JSON-RPC; user-visible behaviour                                                                 | Linux CI: `tauri-driver` (port 4444). macOS local: Appium Mac2 (port 4723). See [`docs/E2E-TESTING.md`](./E2E-TESTING.md). |
| **Manual smoke**     | `docs/RELEASE-MANUAL-SMOKE.md` (see #971)                                                                                                             | OS-level surfaces drivers cannot assert: TCC permission prompts, Gatekeeper, code signing, DMG install, OS-native toasts                        | Human at release-cut, signed off in release PR                                                                             |

---

## Decision tree — where does my test go?

```text
Is the change behind the JSON-RPC boundary (in `src/`)?
├─ YES — does it cross domains or talk to external services?
│   ├─ YES → Rust integration (tests/*.rs)
│   └─ NO  → Rust unit (next to source)
└─ NO — change is in `app/`
    ├─ Is it a pure function, hook, slice, or component in isolation?
    │   └─ YES → Vitest unit (*.test.tsx co-located)
    └─ Is it user-visible AND it crosses UI ⇄ Tauri ⇄ sidecar ⇄ JSON-RPC?
        ├─ YES → WDIO E2E (app/test/e2e/specs/*.spec.ts)
        └─ Is it OS-level (TCC, Gatekeeper, install, OS toasts)?
            └─ YES → Manual smoke checklist
```

If a change touches more than one of these, write a test in **each** layer it touches. Don't substitute one for another.

---

## Failure-path requirement

Every feature leaf in the coverage matrix must have **at least one failure / edge** assertion in addition to the happy path. Examples:

- File-write tool: happy = wrote bytes; failure = path-restriction denial.
- OAuth flow: happy = token issued; edge = expired refresh token recovery.
- Memory store: happy = stored + recalled; edge = forget-then-recall returns nothing.

A spec that asserts only the happy path is incomplete.

---

## Mock policy

- **No real network in unit / integration / E2E.** Use the shared mock backend (`scripts/mock-api-core.mjs`, `scripts/mock-api-server.mjs`, `app/test/e2e/mock-server.ts`).
- Admin endpoints for tests: `GET /__admin/health`, `POST /__admin/reset`, `POST /__admin/behavior`, `GET /__admin/requests`.
- **External services** (Telegram, Slack, Gmail, Notion, Ollama, OpenAI, etc.) are stubbed at the mock backend level; tests assert the request shape via `getRequestLog()`.
- The only acceptable exception is a documented release-cut manual smoke step.

---

## Determinism rules

- No wall-clock waits — use `waitForApp`, `waitForAppReady`, `waitForWebView` helpers, or explicit element-readiness predicates.
- No shared filesystem state — every E2E spec runs inside an isolated `OPENHUMAN_WORKSPACE` (created/cleaned by `app/scripts/e2e-run-spec.sh`).
- No order-dependent specs — each spec must pass when run alone.
- No reliance on absolute coordinates or animation timing.
- No real keyboard via `browser.keys()` on tauri-driver — synthesize via `browser.execute(...)` (see `command-palette.spec.ts` for the pattern).

---

## What the existing harness gives you

- **Mock backend bootstrapping**: `startMockServer` / `stopMockServer` in `app/test/e2e/mock-server.ts`.
- **Auth shortcut**: `triggerAuthDeepLink` / `triggerAuthDeepLinkBypass` in `helpers/deep-link-helpers.ts` skips real OAuth.
- **Element helpers**: `clickNativeButton`, `waitForWebView`, `clickToggle` in `helpers/element-helpers.ts` — use these instead of raw `XCUIElementType*` selectors.
- **Shared flows**: `completeOnboardingIfVisible`, `navigateViaHash`, `navigateToSkills`, `walkOnboarding` in `helpers/shared-flows.ts`.
- **Core RPC from spec**: `callOpenhumanRpc` in `helpers/core-rpc.ts` — drives the sidecar directly when a UI step would be brittle.
- **Platform guards**: `isTauriDriver`, `isMac2`, `supportsExecuteScript` in `helpers/platform.ts`.
- **Artifact capture on failure**: `captureFailureArtifacts` runs from `wdio.conf.ts` — screenshots + DOM dumps land under `app/test/e2e/artifacts/`.

---

## Naming + structure conventions

- WDIO specs: `<feature-area>-flow.spec.ts` for end-to-end product flows; `<feature>.spec.ts` for narrower surfaces.
- Vitest co-location: prefer `Component.tsx` + `Component.test.tsx` siblings; use `__tests__/` only when grouping multiple related tests.
- Rust integration tests: snake_case file matching the surface — `<feature>_e2e.rs` for JSON-RPC-driven flows, `<feature>_integration.rs` for cross-domain.
- Each `describe` / `mod tests` block maps to a feature-list ID range — link the matrix row in a comment if the mapping is non-obvious.

---

## Pre-merge gates

Run before opening a PR. CI runs the same set, but local runs are faster:

```bash
# Rust core
cargo fmt --check
cargo check --manifest-path Cargo.toml
cargo clippy --manifest-path Cargo.toml -- -D warnings
cargo test --manifest-path Cargo.toml

# Tauri shell
cargo check --manifest-path app/src-tauri/Cargo.toml

# Frontend
pnpm typecheck
pnpm lint
pnpm format:check
pnpm test:unit

# Rust integration with mock backend
pnpm test:rust

# E2E (slow — run when behaviour changes user-visibly)
pnpm test:e2e:build
bash app/scripts/e2e-run-spec.sh test/e2e/specs/<your-spec>.spec.ts <id>
```

---

## Not driver-automatable — manual smoke required

Some surfaces cannot be driven by WDIO / Appium because they cross OS-level trust boundaries or hardware paths. These ship with a documented manual smoke checklist (see #971) and a sign-off on the release PR:

- macOS TCC permission prompts (Accessibility, Input Monitoring, Screen Recording, Microphone)
- Gatekeeper signature validation on first launch
- Code-sign integrity (`codesign --verify --deep --strict`)
- DMG install / drag-to-Applications flow
- Auto-update download + relaunch
- OS-native notification toasts on Linux CI (no display server visible to the driver beyond Xvfb)

If a feature has no automated coverage AND is not on the manual smoke list, treat it as untested — open a coverage gap.

---

## Coverage matrix as the contract

Every feature leaf in the [coverage matrix](./TEST-COVERAGE-MATRIX.md) maps to:

1. A test path or paths, **or**
2. A justified `🚫` with a manual-smoke entry.

When you add / remove / rename a feature, **update the matrix row in the same PR**. CI will guard this contract once #965 lands.

---

## When in doubt

- Push the test as low in the layer stack as possible (Rust unit > Rust integration > Vitest > WDIO). Lower layers are faster, more deterministic, and cheaper to run.
- WDIO is for behaviours that genuinely cross UI ⇄ Tauri ⇄ sidecar ⇄ JSON-RPC. Don't drive a unit-testable concern through WDIO just because the UI exists.
- A failing happy path is a regression. A missing failure-path test is a gap. Both are bugs.
