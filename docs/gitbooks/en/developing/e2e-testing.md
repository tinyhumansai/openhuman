---
description: >-
  How to run end-to-end suites: Playwright against the standalone core, WDIO
  through the native Wry host, local scripts, environment variables and CI lanes.
icon: vials
---

# E2E testing

End-to-end coverage uses the Rust mock backend, Playwright in a real browser, and WebDriverIO (WDIO) through the desktop Wry host. Each assertion has one owner: Playwright owns browser-rendered behavior; WDIO owns behavior that requires the native process, window, operating system or Tauri IPC. Only the supported Linux desktop harness runs in CI. macOS and Windows desktop jobs remain disabled until native drivers are available.

## Overview

Desktop E2E tests use WebDriverIO (WDIO) to drive the app through a single `tauri-driver` (WebDriver) session against its native Wry/WebKit webview:

| Platform | Driver                         | Port | App format   | Selectors |
| -------- | ------------------------------ | ---- | ------------ | --------- |
| Linux    | tauri-driver + WebKitWebDriver | 4444 | Debug binary | CSS / DOM |

Linux CI drives the debug binary under Xvfb through `tauri-driver`. macOS and Windows have no automated desktop E2E coverage yet. The Chromium-based driver they used is gone, and a native driver (Appium Mac2 on macOS, WinAppDriver on Windows) would have to replace it. `pnpm --filter openhuman-app test:e2e:build` still produces a `.app` bundle on macOS for manual testing, but there is no supported automated session there.

---

## Quick start

### Linux

```bash
# Build the E2E app
pnpm --filter openhuman-app test:e2e:build

# Run one native spec through the tauri-driver session
pnpm --filter openhuman-app test:e2e:session

# Run the native-only flows in the four desktop suite groups
pnpm --filter openhuman-app test:e2e:all:flows

# List the Playwright units assigned to lane 1 without launching a core
bash app/scripts/e2e-web-shards.sh --lane=1 --lanes=4 --list

# Run that Playwright lane (fresh core and workspace for each unit)
bash app/scripts/e2e-web-shards.sh --lane=1 --lanes=4

# Check that spec basenames have one runner owner
pnpm e2e:ownership:check

# Run a single spec
bash app/scripts/e2e-run-spec.sh test/e2e/specs/smoke.spec.ts smoke
```

`app/scripts/e2e-run-session.sh` starts `tauri-driver` on `TAURI_DRIVER_PORT`
(default `4444`) with `WebKitWebDriver` as its native driver, waits for its
`/status` endpoint, then runs WDIO against `app/test/wdio.conf.ts`. On
headless Linux the app itself runs under Xvfb for a virtual display; the
driver process does not need one.

### Docker on macOS (Linux harness locally)

To run the same Linux harness from macOS, use Docker.

```bash
# Build and run all E2E flows
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

`app/test/e2e/helpers/platform.ts` exports two functions, both hardcoded to `true`: `isTauriDriver()` and `supportsExecuteScript()`. Every session exposes the webview DOM and supports `browser.execute()`, so specs that still branch on either check always take the DOM path. Treat them as compatibility shims, not as platform detection.

### Element helpers

`app/test/e2e/helpers/element-helpers.ts` provides one API over the webview DOM:

| Helper                    | Behavior                                     |
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

- Primary path: `browser.execute(window.__simulateDeepLink(url))`. It works against the webview on every platform tauri-driver supports.
- macOS-only fallbacks: the `macos: deepLink` extension command, then `open -a ...`. CI does not exercise these, because macOS has no automated desktop session.
- Linux has no shell fallback. `xdg-open openhuman://...` needs a `.desktop` file that registers the URL scheme, and the CI container does not have one. If the webview simulate call fails there, `triggerDeepLink` throws immediately.

For release candidates, also run one manual secondary-instance smoke test on Linux
or macOS when you touch single-instance or deep-link startup code. It exercises
`tauri-plugin-single-instance`, which OpenHuman registers with the
`deep-link` feature.

1. Launch OpenHuman normally and leave it running.
2. Trigger `openhuman://auth?token=e2e-token&key=auth` through the OS opener.
3. Confirm the already-running window receives the callback instead of a
   second app instance starting.
4. Confirm the secondary process exits cleanly.

This catches a second instance that starts, or exits with an
error, before Tauri's deep-link forwarding path is installed.

### Writing cross-platform specs

1. Use the helpers in `element-helpers.ts`. Never use raw `XCUIElementType*` selectors in specs.
2. Use `clickNativeButton(text)` instead of inline button-clicking code.
3. Use `hasAppChrome()` instead of checking for `XCUIElementTypeMenuBar`.
4. Use `waitForWebView()` instead of checking for `XCUIElementTypeWebView`.
5. For macOS-only tests, use `process.platform` guards or separate spec files.
6. Use `navigateViaHash(route)` for hash routes. It waits for the hash, `document.readyState` and a mounted React root before it returns. After onboarding, `walkOnboarding()` also waits for `#/home` and a Home-page marker before specs navigate elsewhere.

---

## Environment variables

| Variable                    | Default    | Description                                                            |
| --------------------------- | ---------- | ---------------------------------------------------------------------- |
| `TAURI_DRIVER_PORT`         | `4444`     | Port `tauri-driver` listens on; `wdio.conf.ts` connects here           |
| `E2E_MOCK_PORT`             | `18473`    | Mock backend server port                                               |
| `E2E_PORT_BASE`             | unset      | Web lane port block: mock `base`, core `base+1`, web host `base+2`     |
| `OPENHUMAN_WORKSPACE`       | (temp dir) | App workspace directory                                                |
| `OPENHUMAN_SERVICE_MOCK`    | `0`        | Enable service mock mode                                               |
| `OPENHUMAN_E2E_MODE`        | unset      | Enables destructive test-support RPCs; the E2E runner sets this to `1` |
| `OPENHUMAN_E2E_AUTH_BYPASS` | unset      | Enable JWT bypass auth                                                 |
| `DEBUG_E2E_DEEPLINK`        | (verbose)  | Set to `0` to silence deep link logs                                   |
| `E2E_FORCE_CARGO_CLEAN`     | unset      | Force cargo clean before E2E build                                     |

Two web E2E sessions on one machine need separate ports. The lane refuses to start when any of its three ports is already listening. Its readiness probes are ordinary HTTP GETs that another session's mock, core and web host would answer just as happily, and `openhuman-core run` falls back to a neighboring port instead of exiting, so it could stay alive on a port nothing probes. Set `E2E_PORT_BASE` for the build and the run alike. The mock and core ports are compiled into the bundle and cannot be changed afterwards.

```bash
E2E_PORT_BASE=31000 pnpm --filter openhuman-app test:e2e:web
```

---

## CI workflows

### Push / PR checks

The default pull-request gate is `.github/workflows/ci-fast-hosted.yml`: quality
checks plus the complete unit-test suites for each changed area. No E2E suite
runs on a pull request to `main`.

`.github/workflows/ci-full.yml` runs on pull requests targeting `release` and
on every push to it. Its final gate requires the Playwright matrix, native
desktop suite and Rust E2E:

| Lane              | Job                                   | Platform                                                                            |
| ----------------- | ------------------------------------- | ----------------------------------------------------------------------------------- |
| Rust mock backend | `rust-e2e`                            | Linux                                                                               |
| Playwright web    | `playwright-e2e`                      | Four required Linux jobs, 16 ordinary shards per job plus round-robin special cases |
| Desktop           | `e2e-desktop`, via `e2e-reusable.yml` | Four Linux suite groups                                                             |

Playwright builds its web bundle and standalone core once, then uploads the same artifact to all four required lanes. The required selection runs 110 of the 111 discovered specs: all 48 active browser owners from the consolidation inventory and their assertion recipients remain required, as do every other healthy browser spec. It schedules 64 ordinary shards and nine isolated cases (Connections aliases, Connections deep links, and seven Skills search/install cases), each with its own core and workspace: 73 required units total. A separate `playwright-legacy-diagnostic` job runs the unchanged `aui-tool-result-elements.spec.ts` as one file-atomic unit. Its component compatibility blocker is tracked in [issue #236](https://github.com/senamakel/openhuman/issues/236#issuecomment-6284472501). Only this diagnostic job is advisory; it is excluded from the final gate, while all four required lanes remain strict. The scheduler continues through assigned units to collect failures and exits nonzero if any required unit fails. CI Full therefore runs 74 fresh Playwright units across the required and diagnostic jobs, without claiming the legacy diagnostic passed. Manual Playwright selects all 111 specs strictly, using the same 64 shards and nine isolated cases (73 units); it has no advisory selection.

The desktop lane calls the reusable workflow with `run_linux: true`, `run_macos: false` and `run_windows: false`, so only the Linux harness runs. Linux uses four fresh WDIO sessions: `foundation` (`auth,navigation,system`), `chat` (`chat,skills,journeys`), `connections` (`providers,provider-web,connectors`), and `settings` (`settings,notifications`). Payments and webhooks have no native flows after their browser owners moved to Playwright, so they are absent from the native shard assignments. The consolidation reviewed 51 same-basename native/Playwright pairs; it retained only five narrowly split mixed cases and retired one obsolete offline-banner pair. The native suite also retains 24 preexisting native-only files outside that duplicate inventory, plus `tauri-commands.spec.ts`, whose browser bootstrap checks moved to the Playwright smoke owner while its native IPC checks remain. The 24 native-only files were not assessed for whether they require the desktop host. The reusable workflow applies the same group definitions to its macOS and Windows full-suite jobs, but those jobs are currently disabled.

macOS and Windows desktop E2E therefore have no scheduled coverage on any branch. `.github/workflows/e2e.yml` is a manual dispatch whose `run_macos` and `run_windows` inputs also default to `false`. The older Chromium-based driver depended on CEF/CDP and is unavailable under Wry; do not describe those jobs as active coverage.

### Runner ownership and mixed host cases

Every spec file belongs to one runner. Put browser-rendered UI, browser routes,
forms, chat rendering and settings persistence in `app/test/playwright/specs/`.
Keep `app/test/e2e/specs/` for checks that need a live desktop process, a real
Tauri command, OS notification permission, native deep-link launch, window
behavior or a core restart. A JSON-RPC call by itself does not make a test a
desktop test. Keep shared element actions in
`app/test/e2e/helpers/element-helpers.ts`.

Five files intentionally exist in both runner directories because each side
proves a different contract:

| File                               | Native WDIO assertion                                                                   | Playwright assertion                                                         |
| ---------------------------------- | --------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| `smoke.spec.ts`                    | Live Wry window, app chrome and normal embedded-core startup                            | Browser root/chrome, standalone core bootstrap URL/token and RPC             |
| `login-flow.spec.ts`               | Desktop window and deep-link token handling, including invalid, expired or bypass cases | Browser callback entry, token/profile outcome, browser bypass and onboarding |
| `auth-access-control.spec.ts`      | Desktop reauthentication and native auth-expiry event delivery                          | Account/billing/logout/revocation UI and authoritative auth state            |
| `notifications.spec.ts`            | OS permission state and real notification plugin calls                                  | Rendered notification title/body and System Events/empty Alerts UI           |
| `settings-data-management.spec.ts` | Reset IPC, user-data wipe and embedded-core restart                                     | Clear-data confirmation and cancellation UI                                  |

The executable guard `pnpm e2e:ownership:check` rejects every other duplicate,
stale exception or missing file. Before adding a spec, decide which runtime owns
the behavior, search both spec directories for an equivalent flow (including
renamed files), and add it only to the owning suite. If host and browser cases
cover distinct behavior, name and document both responsibilities explicitly;
never keep a second full copy as extra confidence.

The current inventory has 30 native spec files and 111 Playwright spec files,
with only the five mixed files above shared by basename. The 24 preexisting
native-only files were outside the duplicate inventory and were not assessed
for whether they need a desktop host; `tauri-commands.spec.ts` retains the
native IPC checks described above.
The offline-banner E2Es were retired because their only UI consumer was removed
before this baseline; focused connectivity-selector and listener unit tests
retain the device/backend distinction, and no E2E banner coverage is claimed.

---

## Troubleshooting

### Linux: "WebView not ready" timeout

This usually means `tauri-driver` never reached its `/status` endpoint, or the app crashed before mounting its webview. Use `app/scripts/e2e-run-session.sh`, which starts `tauri-driver` and waits on that endpoint before invoking WDIO, rather than driving the app by hand.

Ensure `DISPLAY` is set and Xvfb is running:

```bash
export DISPLAY=:99
Xvfb :99 -screen 0 1280x1024x24 &
```

Also make sure dbus is running (webkit2gtk needs it):

```bash
eval $(dbus-launch --sh-syntax)
```

### Linux: `tauri-driver` or `WebKitWebDriver` not found

Install `tauri-driver` with `cargo install tauri-driver`, and make sure `WebKitWebDriver` is on `PATH` (on Debian and Ubuntu it ships in `webkit2gtk-driver`) or point `WEBKIT_WEBDRIVER` at its location. `e2e-run-session.sh` defaults to `/usr/bin/WebKitWebDriver`.

### macOS: Deep links not working in `tauri dev`

Deep links require a `.app` bundle. Use `pnpm tauri build --debug --bundles app` instead.

### Docker: Build is slow on first run

The first Docker build compiles Rust and installs the E2E harness dependencies. Subsequent runs use cached layers. Docker volumes cache the Cargo registry and git sources.

## Spec: notifications

The mixed notification cases have separate owners. Playwright verifies the
visible notification title/body and the System Events/empty Alerts UI. WDIO
only checks native permission-state values and calls the operating-system
notification plugin. Run the browser owner through the lane scheduler or run
the native owner with:

```bash
bash app/scripts/e2e-run-spec.sh test/e2e/specs/notifications.spec.ts notifications
```

The native suite uses a live `tauri-driver` session; it does not own ordinary
notification-list RPC or page-rendering assertions. The former connectivity
banner E2Es were retired because source history removed their only UI consumer
before the baseline; focused selector and listener unit tests still cover the
device/backend connectivity distinction. No browser banner coverage is claimed.

---

## Agent-observable artifact flow

### Starting a composer test

Chat drafts persist per thread. Before a spec types its initial prompt, call
`replaceChatComposerText(input, prompt)` from
`app/test/playwright/helpers/chat-composer.ts`. It selects and deletes the
restored draft, waits for the empty value, types through keyboard events, and
checks the exact prompt. Use `clearChatComposer(input)` when a case needs an
empty composer. These helpers support both textarea and Lexical surfaces.

Cases that exercise draft restoration or append a follow-up should keep their
draft and drive that behavior explicitly. The helper's browser regressions live
in `app/test/playwright/specs/chat-composer-helper.spec.ts`.

For a canonical, inspectable run that drops screenshots, page-source dumps, and mock request logs on disk:

```bash
bash app/scripts/e2e-agent-review.sh
```

Artifacts land in `app/test/e2e/artifacts/<timestamp>-agent-review/` by default;
set `E2E_ARTIFACT_DIR` to override the path. The script passes that directory
to Playwright as its output path, where the test result and attachments are
saved. The review case attaches checkpoint screenshots, page HTML, and a
redacted mock request log with `testInfo.attach`.
Playwright also retains its configured trace, screenshot, and video artifacts
on failure. Full details and the helper API: [`agent-observability.md`](agent-observability.md).

---

## Rust inference provider E2E

These tests (`tests/inference_provider_e2e.rs`) use wiremock to mock HTTP upstreams and require no live LLM API calls. They cover OpenAI-compat chat, Anthropic auth style, per-model temperature suppression, Ollama local provider, and the `/v1` HTTP endpoint auth layer.

```bash
# Local:
bash scripts/test-rust-inference-e2e.sh

# Via Docker (Linux, same image as CI):
docker compose -f e2e/docker-compose.yml run --rm inference-e2e
```

---

## See also

- [Testing strategy](testing-strategy.md): which layer a given test belongs in.
- [Agent observability](agent-observability.md): the artifact flow the review run writes to.
- [Tauri shell](architecture/tauri-shell.md): the Wry webview `tauri-driver` attaches to.
- [Getting set up](getting-set-up.md): toolchain and submodules a local E2E build needs.
