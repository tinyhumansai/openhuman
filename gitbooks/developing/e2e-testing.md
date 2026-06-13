---
description: End-to-end testing with WDIO + Appium. CI and local setup.
icon: vials
---

# E2E Testing Guide

## Overview

Desktop E2E tests use **WebDriverIO (WDIO)** to drive the Tauri app through Appium:

| Platform                    | Driver          | Port | App format    | Selectors |
| --------------------------- | --------------- | ---- | ------------- | --------- |
| **Linux / Appium Chromium** | Appium Chromium | 4723 | Debug binary  | CSS / DOM |
| **macOS / Appium Chromium** | Appium Chromium | 4723 | `.app` bundle | CSS / DOM |

OpenHuman's desktop app currently uses the CEF runtime (`tauri-runtime-cef`). CI drives the Linux debug binary with Appium's Chromium driver; manual macOS and Windows E2E use the same Chromium-driver backend.

---

## Quick start

### Linux / Appium Chromium

```bash
# Install Appium and the Chromium driver (one-time)
npm install -g appium@3
appium driver install --source=npm appium-chromium-driver

# Build the E2E app
pnpm --filter openhuman-app test:e2e:build

# Run all flows
pnpm --filter openhuman-app test:e2e:all:flows

# Run a single spec
bash app/scripts/e2e-run-spec.sh test/e2e/specs/smoke.spec.ts smoke
```

On headless Linux, the harness runs under **Xvfb** for a virtual display.

### macOS / Appium Chromium

```bash
# Install Appium + Chromium driver (one-time, needs Node 24+)
npm install -g appium@3
appium driver install --source=npm appium-chromium-driver

# Build the .app bundle
pnpm --filter openhuman-app test:e2e:build

# Run all flows
pnpm --filter openhuman-app test:e2e:all:flows
```

### Docker on macOS (Linux harness locally)

Run the same Linux-based harness from macOS using Docker.

```bash
# Build + run all E2E flows
docker compose -f e2e/docker-compose.yml run --rm e2e

# Build the app first (if needed)
docker compose -f e2e/docker-compose.yml run --rm e2e \
  pnpm --filter openhuman-app test:e2e:build

# Run a single spec
docker compose -f e2e/docker-compose.yml run --rm e2e \
  bash app/scripts/e2e-run-spec.sh test/e2e/specs/smoke.spec.ts smoke
```

Requires Docker Desktop or Colima. The repo is bind-mounted so builds persist between runs.

---

## Architecture

### Platform detection

`app/test/e2e/helpers/platform.ts` exports:

- `isTauriDriver()`, legacy shim that now always returns `true` for the DOM-capable Chromium session
- `isMac2()`, legacy shim that now always returns `false`
- `supportsExecuteScript()`, `true` because the Chromium driver supports `browser.execute()` on every platform

### Element helpers

`app/test/e2e/helpers/element-helpers.ts` provides a unified API:

| Helper                    | Appium Chromium                              |
| ------------------------- | -------------------------------------------- |
| `waitForText(text)`       | XPath over DOM text content                  |
| `waitForButton(text)`     | `button` / `[role="button"]` XPath           |
| `clickText(text)`         | Standard `el.click()`                        |
| `clickNativeButton(text)` | Standard `el.click()` on button              |
| `clickToggle()`           | `[role="switch"]` / `input[type="checkbox"]` |
| `waitForWindowVisible()`  | Window handle check                          |
| `waitForWebView()`        | `document.readyState` check                  |
| `hasAppChrome()`          | Window handle check                          |
| `dumpAccessibilityTree()` | HTML page source                             |

### Stable test IDs

Prefer stable `data-testid` hooks for UI affordances that E2E specs click or poll. Use the taxonomy `<surface>-<element>-<id?>`, for example:

- `cron-jobs-panel`, `cron-refresh`
- `cron-job-row-<jobId>`, `cron-job-toggle-<jobId>`, `cron-job-run-<jobId>`, `cron-job-view-runs-<jobId>`, `cron-job-remove-<jobId>`
- `settings-nav-<routeId>`
- `skill-row-<skillId>`, `skill-install-<skillId>`, `skill-uninstall-<skillId>`
- `thread-row-<threadId>`, `new-thread-button`, `send-message-button`
- `onboarding-next-button`

Use `waitForTestId(testId)` and `clickTestId(testId)` from `element-helpers.ts` when a spec targets one of these hooks. Keep text selectors for user-visible copy assertions, not row/action discovery.

### Deep link helpers

`app/test/e2e/helpers/deep-link-helpers.ts` handles auth deep links:

- **Appium Chromium**: `browser.execute(window.__simulateDeepLink(url))` on every platform
- **macOS fallback**: `macos: deepLink` extension command, then `open -a ...`

For release candidates, also run one manual secondary-instance smoke on Linux
or macOS when touching CEF preflight, single-instance, or deep-link startup
code:

1. Launch OpenHuman normally and leave it running.
2. Trigger `openhuman://auth?token=e2e-token&key=auth` through the OS opener.
3. Confirm the already-running window receives the callback and does not start
   a second full CEF instance.
4. Confirm the secondary process exits cleanly without a CEF cache-lock error.

This catches the class of regressions where a secondary process exits during
CEF cache preflight before Tauri's deep-link forwarding path is installed.

### Writing cross-platform specs

1. **Use helpers** from `element-helpers.ts`, never use raw `XCUIElementType*` selectors in specs
2. **Use `clickNativeButton(text)`** instead of inline button-clicking code
3. **Use `hasAppChrome()`** instead of checking for `XCUIElementTypeMenuBar`
4. **Use `waitForWebView()`** instead of checking for `XCUIElementTypeWebView`
5. For macOS-only tests, use `process.platform` guards or separate spec files
6. Use `navigateViaHash(route)` for hash routes; it waits for the hash,
   `document.readyState`, and a mounted React root before returning. After
   onboarding, `walkOnboarding()` also waits for `#/home` plus a Home-page
   marker before specs navigate elsewhere.

---

## Environment variables

| Variable                    | Default    | Description                                                            |
| --------------------------- | ---------- | ---------------------------------------------------------------------- |
| `APPIUM_PORT`               | `4723`     | Appium server port                                                     |
| `E2E_MOCK_PORT`             | `18473`    | Mock backend server port                                               |
| `OPENHUMAN_WORKSPACE`       | (temp dir) | App workspace directory                                                |
| `OPENHUMAN_SERVICE_MOCK`    | `0`        | Enable service mock mode                                               |
| `OPENHUMAN_E2E_MODE`        | unset      | Enables destructive test-support RPCs; the E2E runner sets this to `1` |
| `OPENHUMAN_E2E_AUTH_BYPASS` | unset      | Enable JWT bypass auth                                                 |
| `DEBUG_E2E_DEEPLINK`        | (verbose)  | Set to `0` to silence deep link logs                                   |
| `E2E_FORCE_CARGO_CLEAN`     | unset      | Force cargo clean before E2E build                                     |

---

## CI workflows

### Push / PR checks

The default pull-request gate is `.github/workflows/pr-ci.yml`. It builds one Linux E2E-compatible desktop artifact, then runs the Linux Appium/Chromium `mega-flow` lane and the Playwright web lane in parallel with Rust and coverage jobs.

macOS and Windows desktop E2E do not run on every PR. Use the manually dispatched E2E workflow, or release pretest workflows, when cross-platform desktop signal is needed.

### macOS / Appium Chromium

macOS/Appium Chromium is available for local runs and through the manually dispatched E2E workflow:

1. Installs Appium + Chromium driver
2. Builds the `.app` bundle
3. Runs all E2E flows

---

## Troubleshooting

### Linux: "WebView not ready" timeout

For the default CEF runtime, this usually means a stale local runner is trying to drive a CEF-backed WebView through WebKitWebDriver. Current CI uses the Appium Chromium driver on Linux; use `app/scripts/e2e-run-session.sh` or the PR CI workflow for the supported Linux path.

Ensure `DISPLAY` is set and Xvfb is running:

```bash
export DISPLAY=:99
Xvfb :99 -screen 0 1280x1024x24 &
```

Also ensure dbus is started (required by webkit2gtk):

```bash
eval $(dbus-launch --sh-syntax)
```

### Linux: Appium Chromium driver not found

```bash
npm install -g appium@3
appium driver install --source=npm appium-chromium-driver
```

### macOS: Deep links not working in `tauri dev`

Deep links require a `.app` bundle. Use `pnpm tauri build --debug --bundles app` instead.

### Docker: Build is slow on first run

The first Docker build compiles Rust and installs the E2E harness dependencies. Subsequent runs use cached layers. Cargo registry and git sources are cached via Docker volumes.

## Spec: Notifications

**File**: `app/test/e2e/specs/notifications.spec.ts`

Tests notification RPC methods via the live core sidecar and the Notifications UI page:

- `notification_ingest`, creates a new notification via core RPC
- `notification_list`, verifies the ingested notification is returned
- `notification_mark_read`, marks a notification as read
- `notification_stats`, checks aggregate statistics shape
- UI: Notifications page renders the integration notifications section (`[data-testid="integration-notifications-section"]`)
- UI: Notifications page shows the System Events section (`[data-testid="system-events-section"]`)

**Run**:

```bash
bash app/scripts/e2e-run-spec.sh test/e2e/specs/notifications.spec.ts notifications
```

**Platform note**: RPC tests (`notification_ingest`, `notification_list`, `notification_mark_read`, `notification_stats`) run through the unified Appium Chromium backend. UI assertions require `browser.execute()` support, which the current backend provides on every platform.

---

## Agent-observable artifact flow

For a canonical, inspectable run that drops screenshots, page-source dumps, and mock request logs on disk:

```bash
bash app/scripts/e2e-agent-review.sh
```

Artifacts land in `app/test/e2e/artifacts/<timestamp>-agent-review/`. Full details + helper API: [`AGENT-OBSERVABILITY.md`](AGENT-OBSERVABILITY.md). Any failing test triggers `wdio.conf.ts`'s `afterTest` hook, which writes `failure-*.png` + `failure-*.source.xml` into the same run dir.

---

## Rust inference provider E2E

These tests (`tests/inference_provider_e2e.rs`) use **wiremock** to mock HTTP upstreams and require no live LLM API calls. They cover OpenAI-compat chat, Anthropic auth style, per-model temperature suppression, Ollama local provider, and the `/v1` HTTP endpoint auth layer.

```bash
# Local:
bash scripts/test-rust-inference-e2e.sh

# Via Docker (Linux, same image as CI):
docker compose -f e2e/docker-compose.yml run --rm inference-e2e
```
