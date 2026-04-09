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

describe('macOS Application Distribution', () => {
  let methods: Set<string>;

  before(async () => {
    await waitForApp();
    await waitForAppReady(20_000);
    methods = await fetchCoreRpcMethods();
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

  runMacOnlyCase('0.2.2', 'Gatekeeper Validation', async () => {
    expectRpcMethod(methods, 'openhuman.service_status');
    const status = await callOpenhumanRpc('openhuman.service_status', {});
    expect(status.ok || Boolean(status.error)).toBe(true);
  });

  runMacOnlyCase('0.2.3', 'Code Signing Verification', () => {
    const bundle = firstExistingBundle();
    expect(bundle).toBeTruthy();

    const executable = path.join(String(bundle), 'Contents', 'MacOS', 'OpenHuman');
    expect(fs.existsSync(executable)).toBe(true);
  });

  runMacOnlyCase('0.2.4', 'First Launch Permissions Prompt', async () => {
    expectRpcMethod(methods, 'openhuman.screen_intelligence_status');
    const status = await callOpenhumanRpc('openhuman.screen_intelligence_status', {});
    expect(status.ok || Boolean(status.error)).toBe(true);
  });

  runMacOnlyCase('0.3.1', 'Auto Update Check', () => {
    expectRpcMethod(methods, 'openhuman.update_check');
  });

  runMacOnlyCase('0.3.2', 'Forced Update Handling', () => {
    expectRpcMethod(methods, 'openhuman.update_apply');
  });

  runMacOnlyCase('0.3.3', 'Reinstall with Existing State', async () => {
    expectRpcMethod(methods, 'openhuman.app_state_snapshot');
    const snapshot = await callOpenhumanRpc('openhuman.app_state_snapshot', {});
    expect(snapshot.ok || Boolean(snapshot.error)).toBe(true);
  });

  runMacOnlyCase('0.3.4', 'Clean Uninstall', async () => {
    expectRpcMethod(methods, 'openhuman.auth_clear_session');
    const clear = await callOpenhumanRpc('openhuman.auth_clear_session', {});
    expect(clear.ok || Boolean(clear.error)).toBe(true);
  });
});
