# E2E Testing Guide

## Overview

Desktop E2E tests use **WebDriverIO (WDIO)** to drive the Tauri app via two automation backends:

| Platform | Driver | Port | App format | Selectors |
|----------|--------|------|------------|-----------|
| **Linux (CI default)** | `tauri-driver` | 4444 | Debug binary | CSS / DOM |
| **macOS (local dev)** | Appium Mac2 | 4723 | `.app` bundle | XPath / accessibility |

**Linux is the default CI path** (`ubuntu-22.04`). macOS E2E is available for local development and as an optional CI workflow.

---

## Quick start

### Linux (CI default)

```bash
# Install tauri-driver (one-time)
cargo install tauri-driver

# Build the E2E app
yarn workspace openhuman-app test:e2e:build

# Run all flows
yarn workspace openhuman-app test:e2e:all:flows

# Run a single spec
bash app/scripts/e2e-run-spec.sh test/e2e/specs/smoke.spec.ts smoke
```

On headless Linux (CI), tests run under **Xvfb** for a virtual display.

### macOS (local dev)

```bash
# Install Appium + Mac2 driver (one-time, needs Node 24+)
npm install -g appium
appium driver install mac2

# Build the .app bundle
yarn workspace openhuman-app test:e2e:build

# Run all flows
yarn workspace openhuman-app test:e2e:all:flows
```

### Docker on macOS (Linux E2E locally)

Run the same Linux-based E2E stack from macOS using Docker:

```bash
# Build + run all E2E flows
docker compose -f e2e/docker-compose.yml run --rm e2e

# Build the app first (if needed)
docker compose -f e2e/docker-compose.yml run --rm e2e \
  yarn workspace openhuman-app test:e2e:build

# Run a single spec
docker compose -f e2e/docker-compose.yml run --rm e2e \
  bash app/scripts/e2e-run-spec.sh test/e2e/specs/smoke.spec.ts smoke
```

Requires Docker Desktop or Colima. The repo is bind-mounted so builds persist between runs.

---

## Architecture

### Platform detection

`app/test/e2e/helpers/platform.ts` exports:

- `isTauriDriver()` — `true` on Linux (tauri-driver session)
- `isMac2()` — `true` on macOS (Appium Mac2 session)
- `supportsExecuteScript()` — `true` when `browser.execute()` works (tauri-driver only)

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

1. **Use helpers** from `element-helpers.ts` — never use raw `XCUIElementType*` selectors in specs
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

### Default (every push/PR)

The `e2e-linux` job runs on `ubuntu-22.04`:
1. Installs system deps (webkit2gtk, Xvfb, dbus)
2. Installs `tauri-driver` via cargo
3. Builds the app with mock server URL baked in
4. Runs all E2E flows under Xvfb

### Optional macOS E2E

The `e2e-macos` job runs only via **manual dispatch** (`workflow_dispatch` with `run_macos_e2e: true`):
1. Installs Appium + Mac2 driver
2. Builds the `.app` bundle
3. Runs all E2E flows

---

## Troubleshooting

### Linux: "WebView not ready" timeout

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

Deep links require a `.app` bundle. Use `yarn tauri build --debug --bundles app` instead.

### Docker: Build is slow on first run

The first Docker build compiles Rust + tauri-driver from source. Subsequent runs use cached layers. Cargo registry and git sources are cached via Docker volumes.
