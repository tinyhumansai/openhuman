import { browser, expect } from '@wdio/globals';

import { waitForApp } from '../helpers/app-helpers';
import { waitForTestId } from '../helpers/element-helpers';
import { isTauriDriver } from '../helpers/platform';
import { resetApp } from '../helpers/reset-app';
import { startMockServer, stopMockServer } from '../mock-server';

/**
 * E2E for the global memory-sync schedule control (#3302).
 *
 * Drives the real React UI + in-process core: navigate to Settings → Account →
 * Data Sync, confirm the schedule defaults to "Every 24h", pick the 4h preset,
 * confirm the summary updates, then re-navigate to prove the choice persisted
 * through `config_update_memory_sync_settings` / `config_get_memory_sync_settings`.
 *
 * DOM-testid assertions only run on the tauri-driver (Linux CI) lane; the macOS
 * Appium Mac2 lane self-skips, matching the other DOM-driven intelligence specs.
 */
function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[MemorySyncScheduleE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[MemorySyncScheduleE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

async function gotoMemoryWorkspace(): Promise<void> {
  // The memory-sync schedule control lives on the layman Data Sync page
  // (Settings → Account → Data Sync), which renders MemorySourcesRegistry and
  // its schedule control without needing developer mode.
  await browser.execute(() => {
    window.location.hash = '/settings/memory-sync';
  });
  await browser.pause(2_000);
}

describe('Memory sync schedule', () => {
  before(async function () {
    stepLog('Starting Memory Sync Schedule E2E');
    await startMockServer();
    await waitForApp();
    await resetApp('e2e-memory-sync-user');
  });

  after(async () => {
    await stopMockServer();
  });

  it('defaults to Every 24h and persists a picked preset', async function () {
    if (!isTauriDriver()) {
      this.skip();
      return;
    }

    await gotoMemoryWorkspace();

    // The schedule control renders independent of any connected sources.
    const schedule = await waitForTestId('memory-sync-schedule');
    expect(await schedule.isExisting()).toBe(true);

    // Default (unset) resolves to the 24h preset being selected.
    const preset24h = await waitForTestId('memory-sync-preset-86400');
    expect(await preset24h.getAttribute('aria-checked')).toBe('true');

    const current = await waitForTestId('memory-sync-current');
    expect(await current.getText()).toContain('Every 24h');

    // Pick the 4h preset — writes config_update_memory_sync_settings.
    const preset4h = await waitForTestId('memory-sync-preset-14400');
    await preset4h.click();

    // The summary reflects the new cadence and the 4h preset becomes selected.
    await browser.waitUntil(
      async () => {
        const el = await waitForTestId('memory-sync-current');
        return (await el.getText()).includes('Every 4h');
      },
      { timeout: 10_000, timeoutMsg: 'sync summary did not update to "Every 4h"' }
    );
    expect(
      await (await waitForTestId('memory-sync-preset-14400')).getAttribute('aria-checked')
    ).toBe('true');

    // Navigate away and back to prove the value was persisted server-side.
    await browser.execute(() => {
      window.location.hash = '/settings';
    });
    await browser.pause(1_000);
    await gotoMemoryWorkspace();

    const reloaded = await waitForTestId('memory-sync-current');
    expect(await reloaded.getText()).toContain('Every 4h');
    expect(
      await (await waitForTestId('memory-sync-preset-14400')).getAttribute('aria-checked')
    ).toBe('true');

    // Switch to Manual only and confirm the summary flips.
    const manual = await waitForTestId('memory-sync-preset-0');
    await manual.click();
    await browser.waitUntil(
      async () => {
        const el = await waitForTestId('memory-sync-current');
        return (await el.getText()).includes('Manual only');
      },
      { timeout: 10_000, timeoutMsg: 'sync summary did not update to "Manual only"' }
    );

    stepLog('Memory sync schedule preset + persistence verified');
  });
});
