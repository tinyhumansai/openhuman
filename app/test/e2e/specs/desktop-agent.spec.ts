import { browser, expect } from '@wdio/globals';

import { waitForApp } from '../helpers/app-helpers';
import { textExists, waitForText } from '../helpers/element-helpers';
import { isTauriDriver } from '../helpers/platform';
import { resetApp } from '../helpers/reset-app';
import { startMockServer, stopMockServer } from '../mock-server';

function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[DesktopAgentE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[DesktopAgentE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

describe('Desktop Agent settings panel', () => {
  before(async function () {
    stepLog('Starting Desktop Agent E2E');
    await startMockServer();
    await waitForApp();
    await resetApp('e2e-desktop-agent-user');
  });

  after(async () => {
    await stopMockServer();
  });

  it('renders the Desktop Agent panel with permissions, toggles, and wake-word hint', async function () {
    if (!isTauriDriver()) {
      this.skip();
      return;
    }

    // Load the settings shell first so nested routes are available.
    await browser.execute(() => {
      window.location.hash = '/settings';
    });
    await browser.pause(2_000);

    // Navigate to the nested desktop-agent route. Retry on hash bounce
    // (lazy component load may briefly redirect), mirroring the
    // Screen Intelligence spec.
    for (let attempt = 0; attempt < 3; attempt++) {
      await browser.execute(() => {
        window.location.hash = '/settings/desktop-agent';
      });
      await browser.pause(3_000);
      const h = String(await browser.execute(() => window.location.hash));
      if (h.includes('/settings/desktop-agent')) break;
      stepLog(`hash bounce attempt ${attempt}`, { hash: h });
    }

    const currentHash = await browser.execute(() => window.location.hash);
    stepLog('Navigated to desktop agent route', { currentHash });

    // Title + beta notice render on every platform.
    await waitForText('Desktop Agent', 15_000);
    expect(await textExists('Beta')).toBe(true);

    // Permission checklist (section title is reused from Screen Intelligence) and
    // the Microphone row — the desktop-agent panel surfaces Microphone, which the
    // Screen Intelligence panel does not. Rows render on every platform (non-macOS
    // permissions just show as unsupported).
    expect(await textExists('Permissions')).toBe(true);
    expect(await textExists('Microphone')).toBe(true);

    // Seamless "act without asking" toggle + the relocated always-on listening
    // section with its "Hey Tiny" wake-word hint.
    expect(await textExists('Let the agent act without asking')).toBe(true);
    expect(await textExists('Hey Tiny')).toBe(true);
  });
});
