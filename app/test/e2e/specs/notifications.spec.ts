// @ts-nocheck
import { browser, expect } from '@wdio/globals';

import { waitForApp } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { supportsExecuteScript } from '../helpers/platform';
import { resetApp } from '../helpers/reset-app';
import { startMockServer, stopMockServer } from '../mock-server';

function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[NotificationsE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[NotificationsE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

/**
 * Poll the core ping/about RPC until it responds or the deadline expires.
 * Fails fast if the sidecar is not reachable within the timeout.
 */
async function waitForCoreSidecar(timeout = 30_000): Promise<void> {
  let lastErr: unknown;
  await browser.waitUntil(
    async () => {
      const result = await callOpenhumanRpc('core.ping', {});
      if (result.ok) {
        stepLog('core ready (ping ok)', { result: result.result });
        return true;
      }
      lastErr = result.error;
      return false;
    },
    {
      timeout,
      interval: 1_000,
      timeoutMsg: `Core sidecar not ready after ${timeout}ms: ${String(lastErr)}`,
    }
  );
}

describe('Notifications', () => {
  before(async function () {
    // waitForApp() + waitForCoreSidecar(30s) cannot fit the default 30s Mocha
    // hook budget; raise it like the other real-flow specs do.
    this.timeout(90_000);
    await startMockServer();
    await waitForApp();
    await resetApp('e2e-notifications-user');

    // Fail fast if core sidecar is not up.
    await waitForCoreSidecar(30_000);
  });

  after(async () => {
    await stopMockServer();
  });

  // Rendered notification content and empty-state UI are owned by Playwright.

  it('native notification permission command returns a valid state', async () => {
    if (!supportsExecuteScript()) {
      stepLog('skipping tauri command test — supportsExecuteScript() is false (Appium Mac2)');
      return;
    }

    // E2E command-wiring validation intentionally exercises the low-level
    // invoke bridge from the webview context.
    const state = await browser.execute(async () => {
      const invoker = (window as unknown as { __TAURI_INTERNALS__?: { invoke?: Function } })
        .__TAURI_INTERNALS__?.invoke;
      if (typeof invoker !== 'function') {
        throw new Error('window.__TAURI_INTERNALS__.invoke is not available');
      }
      return await invoker('notification_permission_state');
    });

    stepLog('notification_permission_state result', { state });
    const allowedStates = [
      'granted',
      'denied',
      'not_determined',
      'provisional',
      'ephemeral',
      'unknown',
    ];
    expect(allowedStates.includes(String(state))).toBe(true);
  });

  it('native notification plugin command is callable from webview', async () => {
    if (!supportsExecuteScript()) {
      stepLog('skipping tauri command test — supportsExecuteScript() is false (Appium Mac2)');
      return;
    }

    // E2E command-wiring validation intentionally exercises the low-level
    // invoke bridge from the webview context.
    const result = await browser.execute(async () => {
      const invoker = (window as unknown as { __TAURI_INTERNALS__?: { invoke?: Function } })
        .__TAURI_INTERNALS__?.invoke;
      if (typeof invoker !== 'function') {
        throw new Error('window.__TAURI_INTERNALS__.invoke is not available');
      }
      await invoker('plugin:notification|notify', {
        options: {
          title: 'OpenHuman E2E notification',
          body: 'Verifies the plugin command is wired and callable.',
        },
      });
      return 'ok';
    });

    stepLog('plugin:notification|notify execute result', { result });
    expect(result).toBe('ok');
  });
});
