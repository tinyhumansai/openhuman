// @ts-nocheck
/**
 * Settings → AI & Skills (capability 13.3).
 *
 * Rewritten to follow the cron-jobs-flow reference: one `resetApp(...)` at
 * the top establishes a fresh-install baseline (auth + onboarding via
 * real UI), then each test navigates to a sub-route and asserts the panel
 * actually mounted. No more per-suite ad-hoc auth bootstrapping.
 *
 * Covers:
 *   - 13.3.1 Local AI Model panel renders presets (Balanced / Performance)
 *   - 13.3.2 Tools panel renders at least one tool toggle
 */
import { waitForApp } from '../helpers/app-helpers';
import { textExists, waitForText } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-settings-ai-skills';

describe('Settings - AI & Skills', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('mounts Local AI Model panel and shows presets (13.3.1)', async () => {
    await navigateViaHash('/settings/local-model');

    await waitForText('Local AI Model', 15_000);
    await waitForText('Device Compatibility', 15_000);
    await waitForText('Preset Tiers', 15_000);

    expect(await textExists('Balanced')).toBe(true);
    expect(await textExists('Performance')).toBe(true);
  });

  it('mounts Tools panel and shows skill toggles (13.3.2)', async () => {
    await navigateViaHash('/settings/tools');

    await waitForText('Tools', 15_000);
    const toolVisible = (await textExists('Filesystem')) || (await textExists('Shell'));
    expect(toolVisible).toBe(true);
  });
});
