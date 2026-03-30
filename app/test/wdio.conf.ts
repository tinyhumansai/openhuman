import type { Options } from '@wdio/types';
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const configDir = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(configDir, '..');
const repoRoot = path.resolve(projectRoot, '..');
const tsconfigE2ePath = path.join(projectRoot, 'test', 'tsconfig.e2e.json');

/**
 * Resolve the path to the built Tauri application bundle.
 *
 * On macOS, Appium mac2 driver launches the .app via bundleId or app path.
 * On Windows/Linux, tauri-driver would be used instead (not covered here).
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
    case 'linux':
      return path.join(projectRoot, 'src-tauri', 'target', 'debug', 'alpha-human');
    default:
      throw new Error(`Unsupported platform: ${process.platform}`);
  }
}

export const config: Options.Testrunner = {
  runner: 'local',
  hostname: '127.0.0.1',
  port: 4723, // Appium default port
  specs: ['./test/e2e/specs/**/*.spec.ts'],
  maxInstances: 1, // Tauri apps are single-instance
  capabilities: [
    {
      platformName: 'mac',
      // @ts-expect-error -- Appium capabilities are not in standard WebDriver types
      'appium:automationName': 'Mac2',
      'appium:app': getAppPath(),
      'appium:showServerLogs': true,
    },
  ],
  logLevel: 'warn',
  bail: 0,
  waitforTimeout: 10_000,
  connectionRetryTimeout: 120_000,
  connectionRetryCount: 3,
  // No appium service — appium is started externally via scripts/start-appium.sh
  // so we can control the Node version (appium v3 requires Node >=24).
  framework: 'mocha',
  reporters: ['spec'],
  mochaOpts: {
    ui: 'bdd',
    timeout: 60_000, // App startup can be slow
  },
  autoCompileOpts: { tsNodeOpts: { project: tsconfigE2ePath } },
};
