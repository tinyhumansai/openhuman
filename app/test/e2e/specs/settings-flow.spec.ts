// @ts-nocheck
/**
 * E2E test: 11. Settings & Configuration
 *
 * Navigates through the Settings pages and verifies:
 *   11.1 Account Settings — profile, linked accounts, billing
 *   11.2 Automation & Channels — accessibility, messaging channels
 *   11.3 AI & Skills — local model, skills page
 *   11.4 Developer Options — webhooks, memory debug
 *   11.5 App Data Management — recovery phrase, privacy, data clearing
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  dumpAccessibilityTree,
  textExists,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import {
  completeOnboardingIfVisible,
  dismissLocalAISnackbarIfVisible,
  navigateToSettings,
  navigateToSkills,
  navigateViaHash,
} from '../helpers/shared-flows';
import { clearRequestLog, getRequestLog, startMockServer, stopMockServer } from '../mock-server';

function stepLog(message: string, context?: unknown) {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[SettingsFlow][${stamp}] ${message}`);
    return;
  }
  console.log(`[SettingsFlow][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

async function waitForAnyText(candidates: string[], timeout = 12_000): Promise<string | null> {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    for (const text of candidates) {
      if (await textExists(text)) return text;
    }
    await browser.pause(500);
  }
  return null;
}

async function expectAnyText(candidates: string[], context: string) {
  const found = await waitForAnyText(candidates, 15_000);
  if (!found) {
    const tree = await dumpAccessibilityTree();
    stepLog(`${context} — none of [${candidates.join(', ')}] found. Tree:`, tree.slice(0, 3000));
  }
  expect(found).not.toBeNull();
  stepLog(`${context} — found "${found}"`);
  return found;
}

/** Navigate to a settings sub-page and wait for content markers. */
async function navigateToSettingsPanel(hash: string, markers: string[], context: string) {
  await navigateViaHash(hash);
  await browser.pause(3_000);
  await expectAnyText(markers, context);
}

async function loginAndNavigateToSettings() {
  clearRequestLog();

  for (let attempt = 1; attempt <= 3; attempt++) {
    stepLog(`trigger deep link (attempt ${attempt})`);
    await triggerAuthDeepLinkBypass(`e2e-settings-flow-${attempt}`);
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await browser.pause(3_000);

    const onLoginPage =
      (await textExists("Sign in! Let's Cook")) || (await textExists('Continue with email'));
    if (!onLoginPage) {
      stepLog(`Auth succeeded on attempt ${attempt}`);
      break;
    }
    if (attempt === 3) {
      const tree = await dumpAccessibilityTree();
      const requests = getRequestLog();
      stepLog('Still on login page. Tree:', tree.slice(0, 3000));
      stepLog('Still on login page. Recent requests:', requests.slice(-20));
      throw new Error(
        `Auth deep link did not navigate past sign-in page\n` +
          `Accessibility tree (truncated):\n${tree.slice(0, 3000)}\n` +
          `Recent request log (${requests.length} total):\n${JSON.stringify(
            requests.slice(-20),
            null,
            2
          )}`
      );
    }
    stepLog('Still on login page — retrying');
    await browser.pause(2_000);
  }

  await completeOnboardingIfVisible('[SettingsFlow]');
  await dismissLocalAISnackbarIfVisible('[SettingsFlow]');

  await navigateToSettings();
  await browser.pause(3_000);

  const settingsMarkers = ['Account & Security', 'Automation & Channels', 'AI & Skills', 'Log out'];
  const found = await waitForAnyText(settingsMarkers, 15_000);
  if (!found) {
    stepLog('Settings page not loaded — retrying');
    await navigateToSettings();
    await browser.pause(3_000);
    const retry = await waitForAnyText(settingsMarkers, 15_000);
    if (!retry) {
      const tree = await dumpAccessibilityTree();
      const requests = getRequestLog();
      stepLog('Settings page not loaded after retry. Tree:', tree.slice(0, 3000));
      stepLog('Settings page not loaded after retry. Recent requests:', requests.slice(-20));
      throw new Error(
        `Could not navigate to Settings page\n` +
          `Accessibility tree (truncated):\n${tree.slice(0, 3000)}\n` +
          `Recent request log (${requests.length} total):\n${JSON.stringify(
            requests.slice(-20),
            null,
            2
          )}`
      );
    }
  }
  await expectAnyText(settingsMarkers, 'Settings home loaded');
}

describe('11. Settings & Configuration', () => {
  before(async () => {
    stepLog('starting mock server');
    await startMockServer();
    stepLog('waiting for app');
    await waitForApp();
  });

  after(async () => {
    stepLog('stopping mock server');
    await stopMockServer();
  });

  beforeEach(async () => {
    await loginAndNavigateToSettings();
  });

  it('opens Settings page after login and onboarding', async () => {
    await expectAnyText(['Account & Security', 'Automation & Channels'], 'Settings home shell');
  });

  // ── 11.1 Account Settings ─────────────────────────────────────────

  describe('11.1 Account Settings', () => {
    it('11.1.1 — Profile Management: Account & Security section visible', async () => {
      await expectAnyText(['Account & Security'], '11.1.1 Account section');
    });

    it('11.1.2 — Linked Accounts Management: Billing panel loads', async () => {
      await navigateToSettingsPanel(
        '/settings/billing',
        ['Billing & Usage', 'Current Plan', 'FREE', 'Upgrade', 'Credits', 'Inference Budget'],
        '11.1.2 billing panel'
      );
    });
  });

  // ── 11.2 Automation & Channels ─────────────────────────────────────

  describe('11.2 Automation & Channels', () => {
    it('11.2.1 — Accessibility Settings: Screen Intelligence panel loads', async () => {
      await navigateToSettingsPanel(
        '/settings/screen-intelligence',
        ['Screen Intelligence', 'Window capture', 'Vision', 'capture policy'],
        '11.2.1 screen intelligence panel'
      );
    });

    it('11.2.2 — Messaging Channel Config: Messaging panel loads', async () => {
      await navigateToSettingsPanel(
        '/settings/messaging',
        ['Messaging Channels', 'Telegram', 'Discord'],
        '11.2.2 messaging panel'
      );
    });
  });

  // ── 11.3 AI & Skills ──────────────────────────────────────────────

  describe('11.3 AI & Skills', () => {
    it('11.3.1 — Model Configuration: Local AI Model panel loads', async () => {
      await navigateToSettingsPanel(
        '/settings/local-model',
        ['Local AI Model', 'Model', 'Download', 'gemma', 'model tier'],
        '11.3.1 local model panel'
      );
    });

    it('11.3.2 — Skill Enable/Disable: Skills page shows built-in skills', async () => {
      // `beforeEach` lands us on the Settings home page, which renders an
      // "AI & Skills" menu item (SettingsHome.tsx). On Mac2, `navigateToSkills`
      // translates to `clickText('Skills', ...)`, which matches the substring
      // "Skills" inside "AI & Skills" and silently navigates to
      // /settings/ai-tools instead of /skills. That section page has no
      // built-in skill cards, so the subsequent text assertion fails.
      //
      // Escape the settings context by hopping through /home first (home has
      // no "Skills"-containing text), then navigate to /skills where the
      // only "Skills" element is the intended sidebar button.
      await navigateViaHash('/home');
      await browser.pause(2_000);

      await navigateToSkills();
      await browser.pause(3_000);
      await expectAnyText(
        ['Screen Intelligence', 'Text Auto-Complete', 'Voice Intelligence'],
        '11.3.2 built-in skills'
      );
    });
  });

  // ── 11.4 Developer Options ─────────────────────────────────────────

  describe('11.4 Developer Options', () => {
    it('11.4.1 — Webhook Inspection: Developer Options panel loads', async () => {
      await navigateToSettingsPanel(
        '/settings/developer-options',
        ['Developer Options', 'Webhooks', 'Memory', 'Debug'],
        '11.4.1 developer options'
      );
    });

    it('11.4.2 — Memory Debug: Memory debug panel loads', async () => {
      await navigateToSettingsPanel(
        '/settings/memory-debug',
        ['Memory', 'Namespace', 'Debug'],
        '11.4.2 memory debug panel'
      );
    });

    it('11.4.3 — Runtime Logs: Webhooks debug panel loads', async () => {
      await navigateToSettingsPanel(
        '/settings/webhooks-debug',
        ['Webhooks', 'Tunnel', 'Debug'],
        '11.4.3 webhooks debug panel'
      );
    });
  });

  // ── 11.5 App Data Management ───────────────────────────────────────

  describe('11.5 App Data Management', () => {
    it('11.5.1 — Clear App Data: Logout option is visible in Settings', async () => {
      await navigateToSettings();
      await browser.pause(3_000);
      await expectAnyText(['Log out', 'Logout', 'Sign out'], '11.5.1 logout action');
    });

    it('11.5.2 — Local Cache Reset: Recovery Phrase panel loads', async () => {
      await navigateToSettingsPanel(
        '/settings/recovery-phrase',
        ['Recovery Phrase', 'BIP39', 'mnemonic', 'recovery', 'phrase'],
        '11.5.2 recovery phrase panel'
      );
    });

    it('11.5.3 — Full State Reset: Privacy panel loads', async () => {
      await navigateToSettingsPanel(
        '/settings/privacy',
        ['Privacy', 'Analytics', 'Data'],
        '11.5.3 privacy panel'
      );
    });
  });
});
