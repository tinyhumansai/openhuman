// @ts-nocheck
import { browser, expect } from '@wdio/globals';

import { waitForApp } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { dumpAccessibilityTree, waitForText } from '../helpers/element-helpers';
import { supportsExecuteScript } from '../helpers/platform';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[NotificationsE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[NotificationsE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

function getUnreadCount(stats: Record<string, unknown>): number {
  const candidates = ['unread_count', 'unread', 'total_unread'];
  for (const key of candidates) {
    const value = stats[key];
    if (typeof value === 'number') return value;
  }
  return 0;
}

async function waitForNotificationsSections(timeout = 10_000): Promise<void> {
  await browser.waitUntil(
    async () =>
      (await browser.execute(() => {
        const integration = document.querySelector(
          '[data-testid="integration-notifications-section"]'
        );
        const system = document.querySelector('[data-testid="system-events-section"]');
        return integration !== null && system !== null;
      })) === true,
    { timeout, timeoutMsg: 'Notifications sections did not render in time' }
  );
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

// Module-level capture: ingest returns a server-generated UUID; share it
// across tests so mark_read and list can reference the same notification.
let ingestedNotifId: string | undefined;

describe('Notifications', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    await resetApp('e2e-notifications-user');

    // Fail fast if core sidecar is not up.
    await waitForCoreSidecar(30_000);
  });

  after(async () => {
    await stopMockServer();
  });

  it('notification_ingest creates a new notification via core RPC', async () => {
    // Required params: provider, title, body, raw_payload (no id/category/timestamp_ms).
    const result = await callOpenhumanRpc('openhuman.notification_ingest', {
      provider: 'e2e',
      title: 'E2E Test Notification',
      body: 'Created by the notifications E2E spec',
      raw_payload: {},
    });
    stepLog('notification_ingest result', { ok: result.ok, result: result.result });
    expect(result.ok).toBe(true);
    // handle_ingest returns RpcOutcome::new(..., vec![]) → bare value (no extra .result wrapper)
    const payload = (result.result as any) ?? {};
    expect(payload.skipped).not.toBe(true);
    expect(typeof payload.id).toBe('string');
    ingestedNotifId = payload.id as string;
    stepLog('captured notification id', { id: ingestedNotifId });
  });

  it('notification_list returns the ingested notification', async () => {
    const result = await callOpenhumanRpc('openhuman.notification_list', { limit: 20 });
    stepLog('notification_list result', { ok: result.ok, result: result.result });
    expect(result.ok).toBe(true);

    // handle_list returns bare value → result.result is {items: [...], unread_count: n}
    const items: unknown[] = (result.result as any)?.items ?? [];
    const found = items.some(
      (n: unknown) =>
        typeof n === 'object' &&
        n !== null &&
        (n as Record<string, unknown>)['title'] === 'E2E Test Notification'
    );
    expect(found).toBe(true);
  });

  it('notification_mark_read transitions notification status', async () => {
    const before = await callOpenhumanRpc('openhuman.notification_stats', {});
    expect(before.ok).toBe(true);
    // handle_stats returns bare value → result.result is {total, unread, ...}
    const beforeStats = (before.result as any) ?? {};
    const initialUnread = getUnreadCount(beforeStats);

    // Use the UUID from the ingest test; fall back to a fresh ingest if needed.
    let notifId = ingestedNotifId;
    if (!notifId) {
      stepLog('no cached notifId — ingesting a fresh notification for mark_read');
      const fresh = await callOpenhumanRpc('openhuman.notification_ingest', {
        provider: 'e2e',
        title: 'E2E Mark Read Fallback',
        body: 'Fallback notification for mark_read test',
        raw_payload: {},
      });
      notifId = (fresh.result as any)?.id as string | undefined;
    }
    expect(notifId).toBeDefined();

    const result = await callOpenhumanRpc('openhuman.notification_mark_read', { id: notifId });
    stepLog('notification_mark_read result', { ok: result.ok, result: result.result });
    expect(result.ok).toBe(true);

    const after = await callOpenhumanRpc('openhuman.notification_stats', {});
    expect(after.ok).toBe(true);
    const afterStats = (after.result as any) ?? {};
    const finalUnread = getUnreadCount(afterStats);
    if (initialUnread > 0) {
      expect(finalUnread).toBeLessThan(initialUnread);
    } else {
      expect(finalUnread).toBe(0);
    }
  });

  it('notification_stats returns aggregate statistics', async () => {
    const result = await callOpenhumanRpc('openhuman.notification_stats', {});
    stepLog('notification_stats result', { ok: result.ok, result: result.result });
    expect(result.ok).toBe(true);
    // handle_stats returns bare value → result.result is {total, unread, unscored, ...}
    const stats = (result.result as any) ?? {};
    // Stats must have at least a numeric total or unread count.
    const hasNumericField = Object.values(stats).some(v => typeof v === 'number');
    expect(hasNumericField).toBe(true);
  });

  it('Notifications page renders integration notifications', async () => {
    if (!supportsExecuteScript()) {
      stepLog('skipping UI test — supportsExecuteScript() is false (Appium Mac2)');
      return;
    }

    // Navigate to /notifications via direct hash set — the route exists but
    // may not have a bottom-tab button. Retry the hash set if it bounces.
    for (let attempt = 0; attempt < 3; attempt++) {
      await browser.execute(() => {
        window.location.hash = '/notifications';
      });
      await browser.pause(1_500);
      const h = await browser.execute(() => window.location.hash);
      if (String(h).includes('/notifications')) break;
      stepLog(`hash bounce attempt ${attempt}`, { hash: h });
    }

    const currentHash = await browser.execute(() => window.location.hash);
    stepLog('Notifications route hash', { currentHash });

    // If the route redirected (e.g. auth guard), skip the UI assertions
    // since the RPC tests above already prove the notification backend works.
    expect(String(currentHash)).toContain('/notifications');

    await waitForNotificationsSections(10_000);

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
    await waitForText('E2E Test Notification', 8_000);
    await waitForText('Created by the notifications E2E spec', 8_000);
  });

  it('Notifications page shows System Events section', async () => {
    if (!supportsExecuteScript()) {
      stepLog('skipping UI test — supportsExecuteScript() is false (Appium Mac2)');
      return;
    }

    await navigateViaHash('/notifications');
    await waitForNotificationsSections(10_000);

    const sectionVisible = await browser.execute(() => {
      const el = document.querySelector('[data-testid="system-events-section"]');
      return el !== null;
    });

    if (!sectionVisible) {
      const tree = await dumpAccessibilityTree();
      stepLog('system-events-section not found', { tree: tree.slice(0, 4000) });
    }
    expect(sectionVisible).toBe(true);

    // The heading text and empty state — the section renders t('alerts.title') = 'Alerts'
    // and t('alerts.empty') = 'No alerts yet' when no system notifications are queued.
    await waitForText('Alerts', 8_000);
    await waitForText('No alerts yet', 8_000);
  });

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
