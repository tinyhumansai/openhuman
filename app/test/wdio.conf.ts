import type { Options } from '@wdio/types';
import path from 'path';
import { fileURLToPath } from 'url';

import { captureFailureArtifacts } from './e2e/helpers/artifacts';

/**
 * Unified WDIO config — single Appium Chromium-driver session that attaches
 * to the running CEF app over its remote-debugging port (CDP).
 *
 * One automation backend on every platform:
 *
 *   macOS / Linux / Windows  →  Appium Chromium driver  →  CEF :19222
 *
 * The runner script (`scripts/e2e-run-session.sh`) is responsible for:
 *   1. Launching the built CEF app binary.
 *   2. Waiting until `http://127.0.0.1:19222/json/version` responds (CDP up).
 *   3. Starting Appium with the `chromium` driver installed.
 *   4. Invoking `wdio` against this config.
 *
 * WDIO creates ONE session per worker. With `maxInstances: 1` and no
 * cross-spec teardown, all specs run sequentially in the same session,
 * against the same app process — no restart cost between spec files.
 * Tests are intentionally order-dependent: state from spec N flows into
 * spec N+1. Each spec is responsible for any reset it requires.
 */

const configDir = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(configDir, '..');
const tsconfigE2ePath = path.join(projectRoot, 'test', 'tsconfig.e2e.json');
const testSpecsPath = path.join(projectRoot, 'test', 'e2e', 'specs', '**', '*.spec.ts');

const APPIUM_PORT = parseInt(process.env.APPIUM_PORT || '4723', 10);
const CEF_CDP_HOST = process.env.CEF_CDP_HOST || '127.0.0.1';
const CEF_CDP_PORT = parseInt(process.env.CEF_CDP_PORT || '19222', 10);

// appium-chromium-driver advertises support for platformName ∈ {windows, mac, linux}
// (not "chromium" — that's only the automationName). Pick the actual OS so the
// capability negotiation succeeds.
function platformNameForHost(): 'mac' | 'linux' | 'windows' {
  if (process.platform === 'darwin') return 'mac';
  if (process.platform === 'win32') return 'windows';
  return 'linux';
}

export const config: Options.Testrunner & Record<string, unknown> = {
  runner: 'local',
  hostname: '127.0.0.1',
  port: APPIUM_PORT,
  path: '/',
  specs: [testSpecsPath],
  rootDir: projectRoot,
  // Single session — Tauri+CEF is one app instance.
  maxInstances: 1,
  capabilities: [
    {
      platformName: platformNameForHost(),
      'appium:automationName': 'Chromium',
      // The runner downloads a chromedriver whose major matches CEF's
      // bundled Chromium and exports its path here. If unset, Appium falls
      // back to its bundled chromedriver — which usually drifts ahead of
      // CEF and produces a "ChromeDriver only supports Chrome version N"
      // session-creation error.
      //
      // Appium chromium driver names this capability `executable` (see
      // appium-chromium-driver/build/lib/desired-caps.js), not the more
      // common Chrome-driver name `chromedriverExecutable`.
      ...(process.env.E2E_CHROMEDRIVER_PATH
        ? { 'appium:executable': process.env.E2E_CHROMEDRIVER_PATH }
        : {}),
      'goog:chromeOptions': {
        // Attach to the already-running CEF process. chromedriver will not
        // try to launch its own Chrome — it picks the first page target
        // exposed at this address (which is the main OpenHuman webview).
        debuggerAddress: `${CEF_CDP_HOST}:${CEF_CDP_PORT}`,
      },
    },
  ],
  logLevel: 'warn',
  bail: 0,
  waitforTimeout: 10_000,
  connectionRetryTimeout: 120_000,
  connectionRetryCount: 3,
  framework: 'mocha',
  reporters: ['spec'],
  mochaOpts: {
    ui: 'bdd',
    // Billing/settings flows poll on real timers; keep the generous budget.
    timeout: 120_000,
  },
  autoCompileOpts: { tsNodeOpts: { project: tsconfigE2ePath } },
  /**
   * After the chromedriver session attaches, switch the active window to
   * the main OpenHuman app webview.
   *
   * CEF exposes multiple CDP page targets:
   *   - `about:blank`  — the CEF prewarm hot-loaded child-webview slot
   *                     (see CEF_PREWARM_LABEL in src-tauri/src/lib.rs).
   *   - `OpenHuman` @ `http://tauri.localhost/#/` — the main React app.
   *
   * `debuggerAddress` makes chromedriver attach to the *first* page target,
   * which is `about:blank`. Without this switch, every spec ends up looking
   * at an empty document. We pick the first window whose URL contains
   * `tauri.localhost`, falling back to the first non-`about:blank`.
   */
  before: async function () {
    const handles = await browser.getWindowHandles();
    let target: string | null = null;
    for (const handle of handles) {
      await browser.switchToWindow(handle);
      const url = await browser.getUrl();
      if (url.includes('tauri.localhost')) {
        target = handle;
        break;
      }
    }
    if (!target) {
      for (const handle of handles) {
        await browser.switchToWindow(handle);
        const url = await browser.getUrl();
        if (!url.startsWith('about:')) {
          target = handle;
          break;
        }
      }
    }
    if (target) {
      await browser.switchToWindow(target);
    }
  },
  afterTest: async function (
    test: { title: string; parent?: string },
    _context: unknown,
    result: { passed: boolean; error?: Error }
  ) {
    if (result.passed) return;
    const name = [test.parent, test.title].filter(Boolean).join(' ').trim() || test.title;
    await captureFailureArtifacts(name);
  },
};
