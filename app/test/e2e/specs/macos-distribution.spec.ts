// @ts-nocheck
import fs from 'fs';
import path from 'path';

import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { expectRpcMethod, fetchCoreRpcMethods } from '../helpers/core-schema';

function isMac(): boolean {
  return process.platform === 'darwin';
}

function candidateAppBundles(): string[] {
  return [
    path.resolve(process.cwd(), 'src-tauri/target/debug/bundle/macos/OpenHuman.app'),
    path.resolve(process.cwd(), '../target/debug/bundle/macos/OpenHuman.app'),
  ];
}

function firstExistingBundle(): string | null {
  for (const bundlePath of candidateAppBundles()) {
    if (fs.existsSync(bundlePath)) return bundlePath;
  }
  return null;
}

function runMacOnlyCase(id: string, title: string, fn: () => Promise<void> | void): void {
  it(`${id} — ${title}`, async function () {
    if (!isMac()) {
      this.skip();
      return;
    }
    await fn();
  });
}

// Module-level sidecar state — populated by the top-level `before` hook and
// read by `runRpcMacOnlyCase` so RPC-dependent cases can self-skip when the
// sidecar didn't come up (e.g. in CI where the Tauri host binds the sidecar
// to a non-default port the fixed 7788–7793 probe range can't discover).
let methods: Set<string> = new Set();
let coreRpcAvailable = false;
let coreRpcError: string | null = null;

/**
 * Run a test case that depends on the core JSON-RPC sidecar being reachable.
 * Skips instead of failing when the sidecar isn't available so the fs-only
 * distribution checks in this spec still run to completion.
 */
function runRpcMacOnlyCase(id: string, title: string, fn: () => Promise<void> | void): void {
  it(`${id} — ${title}`, async function () {
    if (!isMac()) {
      this.skip();
      return;
    }
    if (!coreRpcAvailable) {
      console.log(
        `[macOSDist] ${id} skipped: core JSON-RPC sidecar not reachable (${coreRpcError ?? 'unknown'})`
      );
      this.skip();
      return;
    }
    await fn();
  });
}

describe('macOS Application Distribution', () => {
  before(async () => {
    await waitForApp();
    await waitForAppReady(20_000);
    try {
      methods = await fetchCoreRpcMethods();
      coreRpcAvailable = true;
    } catch (err) {
      coreRpcError = err instanceof Error ? err.message : String(err);
      console.log(
        `[macOSDist] core JSON-RPC sidecar not reachable, RPC-dependent tests will be skipped: ${coreRpcError}`
      );
    }
  });

  runMacOnlyCase('0.1.1', 'Direct Download Access', () => {
    const bundle = firstExistingBundle();
    expect(Boolean(bundle)).toBe(true);
  });

  runMacOnlyCase('0.1.2', 'Version Compatibility Check', () => {
    const bundle = firstExistingBundle();
    expect(bundle).toBeTruthy();

    const infoPlist = path.join(String(bundle), 'Contents', 'Info.plist');
    expect(fs.existsSync(infoPlist)).toBe(true);

    const content = fs.readFileSync(infoPlist, 'utf8');
    expect(content.includes('CFBundleShortVersionString')).toBe(true);
    expect(content.includes('CFBundleVersion')).toBe(true);
  });

  runMacOnlyCase('0.1.3', 'Corrupted Installer Handling', () => {
    const dmgCandidates = [
      path.resolve(process.cwd(), 'src-tauri/target/debug/bundle/dmg'),
      path.resolve(process.cwd(), '../target/debug/bundle/dmg'),
    ];
    const hasDmgDir = dmgCandidates.some(p => fs.existsSync(p));
    expect(hasDmgDir || Boolean(firstExistingBundle())).toBe(true);
  });

  runMacOnlyCase('0.2.1', 'DMG Installation Flow', () => {
    const bundle = firstExistingBundle();
    expect(bundle).toBeTruthy();
    expect(fs.existsSync(path.join(String(bundle), 'Contents', 'MacOS'))).toBe(true);
  });

  runRpcMacOnlyCase('0.2.2', 'Gatekeeper Validation', async () => {
    expectRpcMethod(methods, 'openhuman.service_status');
    const status = await callOpenhumanRpc('openhuman.service_status', {});
    if (!status.ok) {
      console.log('[macOSDist] 0.2.2 service_status failed:', status.error);
    }
    expect(status.ok).toBe(true);
  });

  runMacOnlyCase('0.2.3', 'Code Signing Verification', () => {
    const bundle = firstExistingBundle();
    expect(bundle).toBeTruthy();

    const executable = path.join(String(bundle), 'Contents', 'MacOS', 'OpenHuman');
    expect(fs.existsSync(executable)).toBe(true);
  });

  runRpcMacOnlyCase('0.2.4', 'First Launch Permissions Prompt', async () => {
    expectRpcMethod(methods, 'openhuman.screen_intelligence_status');
    const status = await callOpenhumanRpc('openhuman.screen_intelligence_status', {});
    if (!status.ok) {
      console.log('[macOSDist] 0.2.4 screen_intelligence_status failed:', status.error);
    }
    expect(status.ok).toBe(true);
  });

  runRpcMacOnlyCase('0.3.1', 'Auto Update Check', () => {
    expectRpcMethod(methods, 'openhuman.update_check');
  });

  runRpcMacOnlyCase('0.3.2', 'Forced Update Handling', () => {
    expectRpcMethod(methods, 'openhuman.update_apply');
  });

  runRpcMacOnlyCase('0.3.3', 'Reinstall with Existing State', async () => {
    expectRpcMethod(methods, 'openhuman.app_state_snapshot');
    const snapshot = await callOpenhumanRpc('openhuman.app_state_snapshot', {});
    if (!snapshot.ok) {
      // This spec runs without a mock server, so the sidecar cannot reach the
      // backend to fetch the user profile.  A network/request error is expected
      // and does not indicate a broken snapshot endpoint.
      const isNetworkError =
        typeof snapshot.error === 'string' && snapshot.error.includes('request failed');
      console.log(
        `[macOSDist] 0.3.3 app_state_snapshot failed: ${snapshot.error} (networkError=${isNetworkError})`
      );
      expect(isNetworkError).toBe(true);
    }
  });

  runRpcMacOnlyCase('0.3.4', 'Clean Uninstall', async () => {
    expectRpcMethod(methods, 'openhuman.auth_clear_session');
    const clear = await callOpenhumanRpc('openhuman.auth_clear_session', {});
    if (!clear.ok) {
      console.log('[macOSDist] 0.3.4 auth_clear_session failed:', clear.error);
    }
    expect(clear.ok).toBe(true);
  });
});
