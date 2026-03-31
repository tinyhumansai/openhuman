import type { Options } from '@wdio/types';
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const configDir = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(configDir, '..');
const repoRoot = path.resolve(projectRoot, '..');
const tsconfigE2ePath = path.join(projectRoot, 'test', 'tsconfig.e2e.json');

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
      // tauri-driver launches the binary directly (not a bundle)
      const candidates = [
        path.join(repoRoot, 'target', 'debug', 'OpenHuman'),
        path.join(projectRoot, 'src-tauri', 'target', 'debug', 'OpenHuman'),
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
 * - Linux: tauri-driver (W3C WebDriver, port 4444)
 * - macOS: Appium Mac2 (XCUITest, port 4723)
 */
function getPlatformCapabilities(): WebdriverIO.Capabilities[] {
  if (process.platform === 'linux') {
    return [
      {
        // @ts-expect-error -- tauri:options is a custom capability
        'tauri:options': {
          application: getAppPath(),
        },
      },
    ];
  }

  // macOS: Appium Mac2
  return [
    {
      platformName: 'mac',
      // @ts-expect-error -- Appium capabilities are not in standard WebDriver types
      'appium:automationName': 'Mac2',
      'appium:app': getAppPath(),
      'appium:showServerLogs': true,
    },
  ];
}

/** Port for the automation driver: tauri-driver (4444) or Appium (4723). */
const driverPort =
  process.platform === 'linux'
    ? parseInt(process.env.TAURI_DRIVER_PORT || '4444', 10)
    : parseInt(process.env.APPIUM_PORT || '4723', 10);

export const config: Options.Testrunner = {
  runner: 'local',
  hostname: '127.0.0.1',
  port: driverPort,
  specs: ['./test/e2e/specs/**/*.spec.ts'],
  maxInstances: 1, // Tauri apps are single-instance
  capabilities: getPlatformCapabilities(),
  logLevel: 'warn',
  bail: 0,
  waitforTimeout: 10_000,
  connectionRetryTimeout: 120_000,
  connectionRetryCount: 3,
  // No appium/tauri-driver service — driver is started externally via scripts.
  framework: 'mocha',
  reporters: ['spec'],
  mochaOpts: {
    ui: 'bdd',
    timeout: 60_000, // App startup can be slow
  },
  autoCompileOpts: { tsNodeOpts: { project: tsconfigE2ePath } },
};
