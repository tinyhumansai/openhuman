// @ts-nocheck
import { browser, expect } from '@wdio/globals';

import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  dumpAccessibilityTree,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { supportsExecuteScript } from '../helpers/platform';
import { completeOnboardingIfVisible, navigateViaHash } from '../helpers/shared-flows';
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
  const deadline = Date.now() + timeout;
  let lastErr: unknown;
  while (Date.now() < deadline) {
    const result = await callOpenhumanRpc('openhuman.about_info', {});
    if (result.ok) {
      stepLog('core sidecar ready', { result: result.result });
      return;
    }
    lastErr = result.error;
    await browser.pause(1_000);
  }
  throw new Error(`Core sidecar not ready after ${timeout}ms: ${String(lastErr)}`);
}

describe('Notifications', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();

    await triggerAuthDeepLinkBypass('e2e-notifications-user');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible('[NotificationsE2E]');

    // Fail fast if core sidecar is not up.
    await waitForCoreSidecar(30_000);
  });

  after(async () => {
    await stopMockServer();
  });

  it('notification_ingest creates a new notification via core RPC', async () => {
    const result = await callOpenhumanRpc('openhuman.notification_ingest', {
      id: 'e2e-notif-001',
      category: 'system',
      title: 'E2E Test Notification',
      body: 'Created by the notifications E2E spec',
      timestamp_ms: Date.now(),
    });
    stepLog('notification_ingest result', { ok: result.ok, result: result.result });
    expect(result.ok).toBe(true);
  });

  it('notification_list returns the ingested notification', async () => {
    const result = await callOpenhumanRpc('openhuman.notification_list', { limit: 20 });
    stepLog('notification_list result', { ok: result.ok, result: result.result });
    expect(result.ok).toBe(true);

    const items: unknown[] =
      result.result?.result?.notifications ?? result.result?.result?.items ?? [];
    const found = items.some(
      (n: unknown) =>
        typeof n === 'object' &&
        n !== null &&
        (n as Record<string, unknown>)['id'] === 'e2e-notif-001'
    );
    expect(found).toBe(true);
  });

  it('notification_mark_read transitions notification status', async () => {
    const result = await callOpenhumanRpc('openhuman.notification_mark_read', {
      id: 'e2e-notif-001',
    });
    stepLog('notification_mark_read result', { ok: result.ok, result: result.result });
    expect(result.ok).toBe(true);
  });

  it('notification_stats returns aggregate statistics', async () => {
    const result = await callOpenhumanRpc('openhuman.notification_stats', {});
    stepLog('notification_stats result', { ok: result.ok, result: result.result });
    expect(result.ok).toBe(true);
    const stats = result.result?.result ?? {};
    // Stats must have at least a numeric total or unread count.
    const hasNumericField = Object.values(stats).some(v => typeof v === 'number');
    expect(hasNumericField).toBe(true);
  });

  it('Notifications page renders integration notifications', async () => {
    if (!supportsExecuteScript()) {
      stepLog('skipping UI test — supportsExecuteScript() is false (Appium Mac2)');
      return;
    }

    await navigateViaHash('/notifications');
    await browser.pause(2_000);

    const currentHash = await browser.execute(() => window.location.hash);
    stepLog('Notifications route hash', { currentHash });
    expect(String(currentHash)).toContain('/notifications');

    // The integration notifications section wraps NotificationCenter.
    const sectionVisible = await browser.execute(() => {
      const el = document.querySelector('[data-testid="integration-notifications-section"]');
      return el !== null;
    });

    if (!sectionVisible) {
      const tree = await dumpAccessibilityTree();
      stepLog('integration-notifications-section not found', { tree: tree.slice(0, 4000) });
    }
    expect(sectionVisible).toBe(true);
  });

  it('Notifications page shows System Events section', async () => {
    if (!supportsExecuteScript()) {
      stepLog('skipping UI test — supportsExecuteScript() is false (Appium Mac2)');
      return;
    }

    await navigateViaHash('/notifications');
    await browser.pause(2_000);

    const sectionVisible = await browser.execute(() => {
      const el = document.querySelector('[data-testid="system-events-section"]');
      return el !== null;
    });

    if (!sectionVisible) {
      const tree = await dumpAccessibilityTree();
      stepLog('system-events-section not found', { tree: tree.slice(0, 4000) });
    }
    expect(sectionVisible).toBe(true);

    // The heading text should also be present.
    await waitForText('System Events', 8_000);
  });

  it('native notification permission command returns a valid state', async () => {
    if (!supportsExecuteScript()) {
      stepLog('skipping tauri command test — supportsExecuteScript() is false (Appium Mac2)');
      return;
    }

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
