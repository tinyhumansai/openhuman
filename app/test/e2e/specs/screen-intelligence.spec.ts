import { browser, expect } from '@wdio/globals';

import { waitForApp } from '../helpers/app-helpers';
import {
  clickButton,
  dumpAccessibilityTree,
  textExists,
  waitForText,
} from '../helpers/element-helpers';
import { isTauriDriver } from '../helpers/platform';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { clearRequestLog, startMockServer, stopMockServer } from '../mock-server';

function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[ScreenIntelligenceE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[ScreenIntelligenceE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

async function waitForCaptureOutcome(timeoutMs = 20_000): Promise<'success' | 'failure'> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (
      (await textExists('Success')) &&
      ((await textExists('windowed')) || (await textExists('fullscreen')))
    ) {
      return 'success';
    }
    if (
      (await textExists('Failed')) ||
      (await textExists('screen recording permission is not granted')) ||
      (await textExists('screen capture is unsupported on this platform')) ||
      (await textExists('screen capture failed'))
    ) {
      return 'failure';
    }
    await browser.pause(500);
  }
  throw new Error('Timed out waiting for screen capture outcome');
}

describe('Screen Intelligence', () => {
  before(async function () {
    stepLog('Starting Screen Intelligence E2E');
    await startMockServer();
    await waitForApp();
    await resetApp('e2e-screen-intelligence-user');
    clearRequestLog();
  });

  after(async () => {
    await stopMockServer();
  });

  it('opens the Screen Intelligence settings route', async function () {
    if (!isTauriDriver()) {
      this.skip();
      return;
    }

    // Load the settings shell first so nested routes are available.
    await browser.execute(() => {
      window.location.hash = '/settings';
    });
    await browser.pause(2_000);

    // Now navigate to the nested screen-intelligence route.
    // Retry if the hash bounces (lazy component load may cause redirect).
    for (let attempt = 0; attempt < 3; attempt++) {
      await browser.execute(() => {
        window.location.hash = '/settings/screen-intelligence';
      });
      await browser.pause(3_000);
      const h = String(await browser.execute(() => window.location.hash));
      if (h.includes('/settings/screen-intelligence')) break;
      stepLog(`hash bounce attempt ${attempt}`, { hash: h });
    }

    const currentHash = await browser.execute(() => window.location.hash);
    stepLog('Navigated to screen intelligence route', { currentHash });

    // The panel renders the "Screen Awareness" title on every platform.
    // The "Permissions" sub-section is only rendered when
    // status.platform_supported is true (macOS) — on Linux/Windows the
    // section is gated out by ScreenIntelligencePanel, so don't assert
    // on it here.
    await waitForText('Screen Awareness', 15_000);
  });

  it('triggers capture test and reaches a stable UI outcome', async function () {
    if (!isTauriDriver()) {
      this.skip();
      return;
    }

    // The capture test UI lives in the debug panel, not the main panel.
    await navigateViaHash('/settings/screen-awareness-debug');
    await waitForText('Screen Awareness', 10_000);

    // The Expand button opens the Debug & Diagnostics section.
    // If not present, the debug panel may already be expanded.
    if (await textExists('Expand')) {
      await clickButton('Expand', 10_000);
    }
    await waitForText('Capture test', 10_000);
    await clickButton('Test capture', 10_000);

    const outcome = await waitForCaptureOutcome();
    stepLog('Capture test outcome', { outcome });

    if (outcome === 'success') {
      const hasPreviewImage = await browser.execute(() => {
        const img = document.querySelector('img[alt="Capture test result"]');
        return !!img && !!img.getAttribute('src');
      });
      expect(hasPreviewImage).toBe(true);
      expect((await textExists('windowed')) || (await textExists('fullscreen'))).toBe(true);
      return;
    }

    const hasFailureGuidance =
      (await textExists('Failed')) ||
      (await textExists('screen recording permission is not granted')) ||
      (await textExists('screen capture is unsupported on this platform')) ||
      (await textExists('screen capture failed'));
    if (!hasFailureGuidance) {
      const tree = await dumpAccessibilityTree();
      stepLog('Capture failure outcome missing expected guidance', { tree: tree.slice(0, 4000) });
    }
    expect(hasFailureGuidance).toBe(true);
  });
});
