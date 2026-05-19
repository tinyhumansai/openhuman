// @ts-nocheck
/**
 * E2E test: Voice mode integration — smoke
 *
 * Covers:
 *   - Authenticating and reaching the home screen
 *   - Navigating to the chat surface (/chat)
 *   - Verifying the text input area renders (default mode)
 *   - Verifying the Voice Settings panel is reachable under Settings
 *
 * NOTE: The explicit voice-mode toggle UI (Input/Reply toggle group with
 * "Text" and "Voice" buttons) was removed in the /chat refactor (see
 * internal PR #717, "voice input mic hidden"). The voice mode flow now
 * lives in the cloud-mic composer (MicComposer) on the /human page.
 * This spec covers the reachable parts of the voice surface.
 *
 * The mock server runs on http://127.0.0.1:18473
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { triggerAuthDeepLink } from '../helpers/deep-link-helpers';
import {
  dumpAccessibilityTree,
  textExists,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { completeOnboardingIfVisible, navigateViaHash } from '../helpers/shared-flows';
import { clearRequestLog, startMockServer, stopMockServer } from '../mock-server';

async function waitForHome(timeout = 20_000) {
  // After auth + onboarding the app lands on /home.
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await textExists('Ask your assistant anything')) return true;
    await browser.pause(700);
  }
  return false;
}

async function waitForAnyText(candidates, timeout = 20_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    for (const t of candidates) {
      if (await textExists(t)) return t;
    }
    await browser.pause(600);
  }
  return null;
}

describe('Voice mode integration', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    clearRequestLog();
  });

  after(async () => {
    await stopMockServer();
  });

  it('authenticates and reaches home, then confirms chat surface is reachable', async function () {
    this.timeout(90_000);

    // --- Authenticate and reach home ---
    await triggerAuthDeepLink('e2e-voice-token');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);

    await completeOnboardingIfVisible('[VoiceModeE2E]');

    const onHome = await waitForHome(20_000);
    if (!onHome) {
      const tree = await dumpAccessibilityTree();
      console.log('[VoiceModeE2E] Home not reached. Tree:\n', tree.slice(0, 4000));
    }
    expect(onHome).toBe(true);

    // --- Navigate to chat and verify text input area ---
    await navigateViaHash('/chat');
    await browser.pause(2_000);

    const hash = await browser.execute(() => window.location.hash);
    expect(String(hash)).toContain('/chat');

    const hasTextInput = await waitForAnyText(
      ['Type a message...', 'Ask the agent anything...', 'Type a message', 'Ask the agent'],
      10_000
    );
    expect(hasTextInput).not.toBeNull();
  });

  it('Voice Settings panel is reachable under Settings', async function () {
    this.timeout(90_000);

    await navigateViaHash('/settings/voice');
    await browser.pause(2_000);

    const hash = await browser.execute(() => window.location.hash);
    expect(String(hash)).toContain('/settings/voice');

    // Voice settings panel should show "Mascot Voice" or "Voice" heading.
    const voiceSettingsVisible = await waitForAnyText(
      ['Mascot Voice', 'Voice Dictation', 'Voice Settings', 'Voice'],
      10_000
    );
    expect(voiceSettingsVisible).not.toBeNull();
  });
});
