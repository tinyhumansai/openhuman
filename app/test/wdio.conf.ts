import type { Options } from '@wdio/types';
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

import { captureFailureArtifacts } from './e2e/helpers/artifacts';

/**
 * WDIO config — a single `tauri-driver` (WebDriver) session against the
 * app's native Wry/WebKit webview.
 *
 * The Appium Chromium-driver backend that attached over CEF's remote-debugging
 * port was removed in #5478: CDP only exists under a Chromium engine, and the
 * app moved to Wry in #5456.
 *
 * The runner script (`scripts/e2e-run-session.sh`) is responsible for:
 *   1. Starting `tauri-driver` and waiting for its `/status` endpoint.
 *   2. Invoking `wdio` against this config.
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

function linuxAppPath(): string {
  const candidate = path.join(projectRoot, 'src-tauri', 'target', 'debug', 'OpenHuman');
  if (fs.existsSync(candidate)) return candidate;
  return candidate;
}

// Admin base for the shared mock backend. The runner exports BACKEND_URL to
// the mock; fall back to the E2E_MOCK_PORT default the runner scripts use.
const MOCK_ADMIN_BASE =
  process.env.BACKEND_URL || `http://127.0.0.1:${process.env.E2E_MOCK_PORT || 18473}`;

// The mock backend carries module-level mutable state (conversations, cron
// jobs, webhook triggers, request log, socket sessions). Specs run in ONE
// ordered session and historically only reset it when a spec *remembered* to
// call `/__admin/reset` in its own hook — so a spec that failed before its
// reset poisoned the next spec file. Reset once at the start of every spec
// file, unconditionally, so no spec can leak mock state into the next one.
// Guarded by file path so nested `describe` blocks inside a file don't wipe
// state mid-file (specs still build up state across their own `it`s).
let lastResetSpecFile: string | null = null;

async function resetMockBackendOncePerSpecFile(specFile: string | undefined): Promise<void> {
  if (!specFile || specFile === lastResetSpecFile) return;
  lastResetSpecFile = specFile;
  try {
    const res = await fetch(`${MOCK_ADMIN_BASE}/__admin/reset`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: '{}',
    });
    if (!res.ok) {
      console.warn(`[wdio] mock /__admin/reset returned ${res.status} for ${specFile}`);
    }
  } catch (err) {
    // Best-effort: some lanes run specs that don't stand up the mock backend.
    // Never fail the run on an unreachable mock — just record it.
    console.warn(`[wdio] mock /__admin/reset skipped for ${specFile}: ${(err as Error).message}`);
  }
}

export const config: Options.Testrunner & Record<string, unknown> = {
  runner: 'local',
  hostname: '127.0.0.1',
  port: parseInt(process.env.TAURI_DRIVER_PORT || '4444', 10),
  path: '/',
  specs: [testSpecsPath],
  rootDir: projectRoot,
  // Single session — the app is one instance.
  maxInstances: 1,
  capabilities: [
    {
      'tauri:options': { application: linuxAppPath() },
      // WDIO's per-capability ceiling is required by the Tauri driver.
      // Without it, the runner can schedule one WebKit session per spec
      // despite the global maxInstances: 1, causing simultaneous app
      // resets and cascading startup timeouts.
      'wdio:maxInstances': 1,
    },
  ],
  logLevel: 'warn',
  // `bail` is the number of failing specs to tolerate before WDIO stops the
  // run. `--bail` on e2e-run-all-flows.sh sets E2E_BAIL_ON_FAILURE=1 so we
  // flip this to 1 (= stop after the first failed spec).
  bail: process.env.E2E_BAIL_ON_FAILURE === '1' ? 1 : 0,
  // Linux shards retry failed specs in e2e-run-all-flows.sh after restarting
  // tauri-driver and the app. Retrying here would reuse the same driver and
  // turn a stuck POST /session into another two-minute timeout.
  specFileRetries: 0,
  specFileRetriesDeferred: true,
  waitforTimeout: 10_000,
  connectionRetryTimeout: 120_000,
  connectionRetryCount: 3,
  framework: 'mocha',
  reporters: ['spec'],
  mochaOpts: {
    ui: 'bdd',
    // Under the native Linux driver, a reset after a tool-heavy spec can wait
    // behind WebKit and core cleanup long enough to exceed one minute. Keep
    // the historical two-minute suite budget; individual polling helpers
    // still keep their own short, diagnostic timeouts.
    timeout: 120_000,
  },
  autoCompileOpts: { tsNodeOpts: { project: tsconfigE2ePath } },
  /**
   * Switch the active window to the main OpenHuman app webview.
   *
   * The driver may hand back a handle for a non-app window, so pick the
   * first whose URL contains `tauri.localhost`, falling back to the first
   * non-`about:` one.
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
  beforeSuite: async function (suite: { file?: string }) {
    // Fires once per Mocha suite. The per-file guard makes the reset run only
    // for the first suite encountered in each spec file.
    await resetMockBackendOncePerSpecFile(suite?.file);
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
