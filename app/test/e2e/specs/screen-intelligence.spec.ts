import { browser, expect } from '@wdio/globals';

import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  clickButton,
  dumpAccessibilityTree,
  hasAppChrome,
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { isTauriDriver } from '../helpers/platform';
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
  before(async () => {
    stepLog('Starting Screen Intelligence E2E');
    await startMockServer();
    await waitForApp();
    clearRequestLog();
  });

  after(async () => {
    await stopMockServer();
  });

  it('authenticates and reaches the app shell', async () => {
    await triggerAuthDeepLinkBypass('e2e-screen-intelligence-user');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    expect(await hasAppChrome()).toBe(true);
  });

  it('opens the Screen Intelligence settings route', async function () {
    if (!isTauriDriver()) {
      this.skip();
      return;
    }

    await navigateViaHash('/settings/screen-intelligence');
    const currentHash = await browser.execute(() => window.location.hash);
    stepLog('Navigated to screen intelligence route', { currentHash });

    expect(currentHash).toContain('/settings/screen-intelligence');
    await waitForText('Screen Intelligence', 10_000);
    await waitForText('Screen Intelligence Policy', 10_000);
    await waitForText('Permissions', 10_000);
  });

  it('triggers capture test and reaches a stable UI outcome', async function () {
    if (!isTauriDriver()) {
      this.skip();
      return;
    }

    if (!(await textExists('Screen Intelligence Policy'))) {
      await navigateViaHash('/settings/screen-intelligence');
      await waitForText('Screen Intelligence Policy', 10_000);
    }

    await clickButton('Expand', 10_000);
    await waitForText('Capture Test', 10_000);
    await clickButton('Test Capture', 10_000);

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
