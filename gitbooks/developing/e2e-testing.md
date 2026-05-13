---
description: End-to-end testing with WDIO + tauri-driver / Appium. CI and local setup.
icon: vials
---

# E2E Testing Guide

## Overview

Desktop E2E tests use **WebDriverIO (WDIO)** to drive the Tauri app via two automation backends:

| Platform | Driver | Port | App format | Selectors |
|----------|--------|------|------------|-----------|
| **Linux / CEF status** | `tauri-driver` | 4444 | Debug binary | CSS / DOM |
| **macOS / Appium** | Appium Mac2 | 4723 | `.app` bundle | XPath / accessibility |

OpenHuman's desktop app currently uses the CEF runtime (`tauri-runtime-cef`). Linux `tauri-driver` talks to WebKitWebDriver / webkit2gtk and cannot drive a CEF-backed WebView, so Linux CEF E2E is disabled in CI until a CEF-compatible driver or replacement harness exists. The supported path today is macOS/Appium for local runs, with manual macOS/Appium workflow runs when that workflow is enabled.

---

## Quick start

### Linux / CEF status

```bash
# Install tauri-driver (one-time)
cargo install tauri-driver

# Build the E2E app
pnpm --filter openhuman-app test:e2e:build

# Run all flows
pnpm --filter openhuman-app test:e2e:all:flows

# Run a single spec
bash app/scripts/e2e-run-spec.sh test/e2e/specs/smoke.spec.ts smoke
```

On headless Linux, the harness runs under **Xvfb** for a virtual display. This path is currently useful only for non-CEF / WebKit-compatible debugging; the default CEF app cannot be automated by WebKitWebDriver.

### macOS / Appium

```bash
# Install Appium + Mac2 driver (one-time, needs Node 24+)
npm install -g appium
appium driver install mac2

# Build the .app bundle
pnpm --filter openhuman-app test:e2e:build

# Run all flows
pnpm --filter openhuman-app test:e2e:all:flows
```

### Docker on macOS (Linux harness locally)

Run the same Linux-based harness from macOS using Docker. The same CEF limitation applies: this is not a supported path for the default CEF runtime until a CEF-compatible driver exists.

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

- `isTauriDriver()`, `true` on Linux (tauri-driver session)
- `isMac2()`, `true` on macOS (Appium Mac2 session)
- `supportsExecuteScript()`, `true` when `browser.execute()` works (tauri-driver only)

### Element helpers

`app/test/e2e/helpers/element-helpers.ts` provides a unified API:

| Helper | Mac2 (macOS) | tauri-driver (Linux) |
|--------|-------------|---------------------|
| `waitForText(text)` | XPath over @label/@value/@title | XPath over DOM text content |
| `waitForButton(text)` | XCUIElementTypeButton XPath | `button` / `[role="button"]` XPath |
| `clickText(text)` | W3C pointer actions | Standard `el.click()` |
| `clickNativeButton(text)` | W3C pointer actions on XCUIElementTypeButton | Standard `el.click()` on button |
| `clickToggle()` | XCUIElementTypeSwitch / XCUIElementTypeCheckBox | `[role="switch"]` / `input[type="checkbox"]` |
| `waitForWindowVisible()` | XCUIElementTypeWindow | Window handle check |
| `waitForWebView()` | XCUIElementTypeWebView | `document.readyState` check |
| `hasAppChrome()` | XCUIElementTypeMenuBar | Window handle check |
| `dumpAccessibilityTree()` | Accessibility XML | HTML page source |

### Deep link helpers

`app/test/e2e/helpers/deep-link-helpers.ts` handles auth deep links:

- **tauri-driver**: `browser.execute(window.__simulateDeepLink(url))` (primary), `xdg-open` (fallback)
- **Appium Mac2**: `macos: deepLink` extension command (primary), `open -a ...` (fallback)

### Writing cross-platform specs

1. **Use helpers** from `element-helpers.ts`, never use raw `XCUIElementType*` selectors in specs
2. **Use `clickNativeButton(text)`** instead of inline button-clicking code
3. **Use `hasAppChrome()`** instead of checking for `XCUIElementTypeMenuBar`
4. **Use `waitForWebView()`** instead of checking for `XCUIElementTypeWebView`
5. For macOS-only tests, use `process.platform` guards or separate spec files

---

## Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `TAURI_DRIVER_PORT` | `4444` | tauri-driver WebDriver port |
| `APPIUM_PORT` | `4723` | Appium server port |
| `E2E_MOCK_PORT` | `18473` | Mock backend server port |
| `OPENHUMAN_WORKSPACE` | (temp dir) | App workspace directory |
| `OPENHUMAN_SERVICE_MOCK` | `0` | Enable service mock mode |
| `OPENHUMAN_E2E_AUTH_BYPASS` | unset | Enable JWT bypass auth |
| `DEBUG_E2E_DEEPLINK` | (verbose) | Set to `0` to silence deep link logs |
| `E2E_FORCE_CARGO_CLEAN` | unset | Force cargo clean before E2E build |

---

## CI workflows

### Push / PR checks

The default `test.yml` workflow runs frontend unit tests and Rust checks. Its Linux `tauri-driver` E2E job is commented out because WebKitWebDriver cannot drive the CEF-backed WebView.

The disabled Linux E2E job used to:
1. Installs system deps (webkit2gtk, Xvfb, dbus)
2. Installs `tauri-driver` via cargo
3. Builds the app with mock server URL baked in
4. Runs all E2E flows under Xvfb

### macOS / Appium

macOS/Appium is the supported automation backend for the current CEF desktop app. Run it locally, or through a manually dispatched macOS workflow when that workflow is enabled:
1. Installs Appium + Mac2 driver
2. Builds the `.app` bundle
3. Runs all E2E flows

---

## Troubleshooting

### Linux: "WebView not ready" timeout

For the default CEF runtime, this usually means the unsupported Linux `tauri-driver` path is trying to drive a CEF-backed WebView through WebKitWebDriver. Use macOS/Appium, or wait for a CEF-compatible Linux driver.

Ensure `DISPLAY` is set and Xvfb is running:
```bash
export DISPLAY=:99
Xvfb :99 -screen 0 1280x1024x24 &
```

Also ensure dbus is started (required by webkit2gtk):
```bash
eval $(dbus-launch --sh-syntax)
```

### Linux: tauri-driver not found

```bash
cargo install tauri-driver
```

### macOS: Deep links not working in `tauri dev`

Deep links require a `.app` bundle. Use `pnpm tauri build --debug --bundles app` instead.

### Docker: Build is slow on first run

The first Docker build compiles Rust + tauri-driver from source. Subsequent runs use cached layers. Cargo registry and git sources are cached via Docker volumes.

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

**Platform note**: RPC tests (`notification_ingest`, `notification_list`, `notification_mark_read`, `notification_stats`) are written for both Linux/tauri-driver and macOS/Appium Mac2, but Linux execution is disabled for the default CEF runtime until a CEF-compatible driver exists. UI assertions (Notifications page sections) require `browser.execute()` support, so they auto-skip on Mac2 when `supportsExecuteScript()` returns `false`.

---

## Agent-observable artifact flow

For a canonical, inspectable run that drops screenshots, page-source dumps, and mock request logs on disk:

```bash
bash app/scripts/e2e-agent-review.sh
```

Artifacts land in `app/test/e2e/artifacts/<timestamp>-agent-review/`. Full details + helper API: [`AGENT-OBSERVABILITY.md`](AGENT-OBSERVABILITY.md). Any failing test triggers `wdio.conf.ts`'s `afterTest` hook, which writes `failure-*.png` + `failure-*.source.xml` into the same run dir.
