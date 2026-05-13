import type { Options } from '@wdio/types';
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

import { captureFailureArtifacts } from './e2e/helpers/artifacts';

const configDir = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(configDir, '..');
const repoRoot = path.resolve(projectRoot, '..');
const tsconfigE2ePath = path.join(projectRoot, 'test', 'tsconfig.e2e.json');
const testSpecsPath = path.join(projectRoot, 'test', 'e2e', 'specs', '**', '*.spec.ts');

/**
 * Resolve the path to the built Tauri application.
 *
 * - macOS: .app bundle for Appium Mac2
 * - Linux: debug binary for tauri-driver
 * - Windows: .exe for tauri-driver
 */
function getAppPath(): string {
  const bundleBases = [
    path.join(projectRoot, 'src-tauri', 'target', 'debug', 'bundle'),
    path.join(repoRoot, 'target', 'debug', 'bundle'),
  ];

  switch (process.platform) {
    case 'darwin': {
      for (const base of bundleBases) {
        const appPath = path.join(base, 'macos', 'OpenHuman.app');
        if (fs.existsSync(appPath)) {
          return appPath;
        }
      }
      return path.join(bundleBases[0], 'macos', 'OpenHuman.app');
    }
    case 'win32':
      return path.join(projectRoot, 'src-tauri', 'target', 'debug', 'OpenHuman.exe');
    case 'linux': {
      // tauri-driver launches the binary directly (not a bundle).
      // Prefer the Tauri build output (src-tauri/target) over the repo-root
      // target/ which may contain a stale core-only binary.
      const candidates = [
        path.join(projectRoot, 'src-tauri', 'target', 'debug', 'OpenHuman'),
        path.join(repoRoot, 'target', 'debug', 'OpenHuman'),
      ];
      for (const candidate of candidates) {
        if (fs.existsSync(candidate)) return candidate;
      }
      return candidates[0];
    }
    default:
      throw new Error(`Unsupported platform: ${process.platform}`);
  }
}

/**
 * Build capabilities for the current platform.
 *
 * - Linux:   tauri-driver (W3C WebDriver, port 4444)
 * - Windows: tauri-driver (W3C WebDriver, port 4444) — same as Linux,
 *            launches the bare .exe rather than a bundle.
 * - macOS:   Appium Mac2 (XCUITest, port 4723)
 */
function getPlatformCapabilities(): Record<string, unknown>[] {
  if (process.platform === 'linux' || process.platform === 'win32') {
    return [{ 'tauri:options': { application: getAppPath() } }];
  }

  // macOS: Appium Mac2
  return [
    {
      platformName: 'mac',
      'appium:automationName': 'Mac2',
      'appium:app': getAppPath(),
      'appium:showServerLogs': true,
    },
  ];
}

/** Port for the automation driver: tauri-driver (4444) or Appium (4723). */
const isTauriDriverHost = process.platform === 'linux' || process.platform === 'win32';
const driverPort = isTauriDriverHost
  ? parseInt(process.env.TAURI_DRIVER_PORT || '4444', 10)
  : parseInt(process.env.APPIUM_PORT || '4723', 10);

export const config: Options.Testrunner & Record<string, unknown> = {
  runner: 'local',
  hostname: '127.0.0.1',
  port: driverPort,
  specs: [testSpecsPath],
  rootDir: projectRoot,
  maxInstances: 1, // Tauri apps are single-instance
  capabilities: getPlatformCapabilities(),
  logLevel: 'warn',
  bail: 0,
  waitforTimeout: 10_000,
  // Linux tauri-driver can take longer to establish the initial session on
  // loaded CI runners; keep macOS defaults while giving Linux more headroom.
  connectionRetryTimeout: isTauriDriverHost ? 240_000 : 120_000,
  connectionRetryCount: isTauriDriverHost ? 5 : 3,
  // No appium/tauri-driver service — driver is started externally via scripts.
  framework: 'mocha',
  reporters: ['spec'],
  mochaOpts: {
    ui: 'bdd',
    timeout: 120_000, // Billing/settings tests need extra time for API polling
  },
  autoCompileOpts: { tsNodeOpts: { project: tsconfigE2ePath } },
  /**
   * Always capture screenshot + page source on failure so agents can
   * inspect what the app looked like the moment the assertion failed.
   */
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
