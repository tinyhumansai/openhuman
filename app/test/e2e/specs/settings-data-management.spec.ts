// @ts-nocheck
/**
 * Settings → Data Management (capability 13.5).
 *
 * The "Full State Reset" test intentionally runs LAST — it logs the user
 * out, so anything that follows would need its own resetApp() pass. We keep
 * this spec self-contained so the suite ordering doesn't matter.
 *
 * Covers the native-only 13.5.3 reset lifecycle. The browser-owned test covers
 * the confirmation dialog and Cancel action.
 */
import fs from 'node:fs';
import path from 'node:path';

import { waitForApp } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import {
  clickSelector,
  clickTestId,
  textExists,
  waitForTestId,
  waitForText,
} from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-settings-data-mgmt';

describe('Settings - Data Management', function () {
  this.timeout(90_000);

  before(async () => {
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('performs Full State Reset (13.5.3)', async function () {
    this.timeout(90_000);
    const workspace = await callOpenhumanRpc('openhuman.test_support_workspace_root', {});
    expect(workspace.ok).toBe(true);
    const workspacePath = workspace.result?.result?.path;
    expect(typeof workspacePath).toBe('string');
    expect(workspace.result?.result?.exists).toBe(true);
    const authBeforeReset = await callOpenhumanRpc('openhuman.auth_get_state', {});
    expect(authBeforeReset.ok).toBe(true);
    const currentUserId = authBeforeReset.result?.result?.userId;
    expect(typeof currentUserId).toBe('string');
    expect(currentUserId.length).toBeGreaterThan(0);
    const frontendUserId = await browser.execute(() => {
      const readState = (
        window as unknown as {
          __OPENHUMAN_CORE_STATE__?: () => { snapshot?: { auth?: { userId?: string | null } } };
        }
      ).__OPENHUMAN_CORE_STATE__;
      return readState?.()?.snapshot?.auth?.userId ?? null;
    });
    expect(frontendUserId).toBe(currentUserId);
    expect(process.env.OPENHUMAN_WORKSPACE).toBeTruthy();
    expect(process.env.HOME).toBeTruthy();
    const resetPaths = await callOpenhumanRpc('openhuman.config_get_data_paths', {
      user_id: currentUserId,
    });
    expect(resetPaths.ok).toBe(true);
    const currentUserDir = resetPaths.result?.result?.current_openhuman_dir;
    const defaultOpenHumanDir = resetPaths.result?.result?.default_openhuman_dir;
    const privateDefaultDir = path.join(process.env.HOME, '.openhuman');
    expect(defaultOpenHumanDir).toBe(privateDefaultDir);
    expect(currentUserDir).toBe(path.join(privateDefaultDir, 'users', currentUserId));
    const sentinelPath = path.join(currentUserDir, 'settings-data-management-sentinel.txt');
    fs.mkdirSync(currentUserDir, { recursive: true });
    fs.writeFileSync(sentinelPath, 'user-scoped clear-data sentinel', { flag: 'wx' });
    expect(fs.readFileSync(sentinelPath, 'utf8')).toBe('user-scoped clear-data sentinel');

    await navigateViaHash('/settings/account');
    await clickTestId('settings-nav-logout-and-clear', 15_000);

    await waitForText('This will sign you out', 5_000);
    await clickSelector('[data-slot="alert-dialog-action"]');
    await browser.waitUntil(async () => !fs.existsSync(sentinelPath), {
      timeout: 60_000,
      interval: 250,
      timeoutMsg: 'reset_local_data did not delete the current user directory sentinel',
    });
    await browser.waitUntil(async () => await textExists('Welcome to OpenHuman'), {
      timeout: 30_000,
      interval: 250,
      timeoutMsg: 'clear-data reset did not reload the signed-out Welcome screen',
    });
    await waitForTestId('welcome-cta-tinyhumans');

    await browser.waitUntil(async () => (await callOpenhumanRpc('core.ping', {})).ok, {
      timeout: 30_000,
      interval: 250,
      timeoutMsg: 'the core did not become healthy after the clear-data restart',
    });
    const auth = await callOpenhumanRpc('openhuman.auth_get_state', {});
    expect(auth.ok).toBe(true);
    expect(auth.result?.result?.isAuthenticated).toBe(false);
    expect(auth.result?.result?.credential).toBeUndefined();
    const ping = await callOpenhumanRpc('core.ping', {});
    expect(ping.ok).toBe(true);
  });
});
