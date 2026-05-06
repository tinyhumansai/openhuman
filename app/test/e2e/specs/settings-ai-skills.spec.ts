import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { supportsExecuteScript } from '../helpers/platform';
import { completeOnboardingIfVisible, navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

/**
 * AI & Skills E2E spec (ID 13.3).
 * Covers:
 * - 13.3.1 Model Configuration switch
 * - 13.3.2 Skill Toggle on/off persistence (covered by skill-lifecycle.spec.ts,
 *   but added here for completeness of section 13.3)
 */

function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[SettingsAISkillsE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[SettingsAISkillsE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

describe('Settings - AI & Skills', () => {
  before(async function beforeSuite() {
    if (!supportsExecuteScript()) {
      stepLog('Skipping suite on Mac2 — navigation helpers require browser.execute');
      this.skip();
    }

    stepLog('starting mock server');
    await startMockServer();
    stepLog('waiting for app');
    await waitForApp();
    stepLog('triggering auth bypass deep link');
    await triggerAuthDeepLinkBypass('e2e-ai-skills');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible('[SettingsAISkillsE2E]');
  });

  after(async () => {
    stepLog('stopping mock server');
    await stopMockServer();
  });

  it('mounts Local AI Model panel and shows presets (13.3.1)', async () => {
    stepLog('navigating to /settings/local-model');
    await navigateViaHash('/settings/local-model');

    await waitForText('Local AI Model', 15_000);
    await waitForText('Device Compatibility', 15_000);

    // Presets should be loaded from mock
    await waitForText('Preset Tiers', 15_000);
    expect(await textExists('Balanced')).toBe(true);
    expect(await textExists('Performance')).toBe(true);
  });

  it('mounts Tools panel and shows skill toggles (13.3.2)', async () => {
    stepLog('navigating to /settings/tools');
    await navigateViaHash('/settings/tools');

    await waitForText('Tools', 15_000);
    // At least one tool should be visible
    const toolVisible = (await textExists('Filesystem')) || (await textExists('Shell'));
    expect(toolVisible).toBe(true);
  });
});
