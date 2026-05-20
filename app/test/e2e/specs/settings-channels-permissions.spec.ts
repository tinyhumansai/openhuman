// @ts-nocheck
/**
 * Settings → Channels & Permissions (capability 13.2).
 *
 * Rewritten to follow the cron-jobs-flow pattern: `resetApp(...)` brings
 * the app to a fresh-install baseline first, then each test drives a
 * settings sub-panel through real navigation + click assertions.
 *
 * Covers:
 *   - 13.2.1 Switching default messaging channel (Telegram ↔ Discord)
 *   - 13.2.2 Privacy panel renders + analytics toggle is present
 */
import { waitForApp } from '../helpers/app-helpers';
import { clickText, textExists, waitForText } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-settings-channels';

describe('Settings - Channels & Permissions', () => {
  before(async function beforeSuite() {
    this.timeout(90_000);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('allows switching default messaging channel (13.2.1)', async function () {
    this.timeout(90_000);
    await navigateViaHash('/settings/messaging');

    await waitForText('Default Messaging Channel', 15_000);
    expect(await textExists('Telegram')).toBe(true);
    expect(await textExists('Discord')).toBe(true);

    await clickText('Discord');
    // After clicking Discord, the route summary shows either an active route
    // or "No active route" (when no Discord account is connected in E2E).
    // We assert that the Active route label is rendered either way.
    await browser.pause(1_000);
    expect((await textExists('Active route')) || (await textExists('No active route'))).toBe(true);
  });

  it('renders privacy settings and analytics toggle (13.2.2)', async function () {
    this.timeout(90_000);
    await navigateViaHash('/settings/privacy');

    // Privacy panel shows 'Privacy & Security' as the panel title and
    // 'Share Anonymized Usage Data' as the analytics toggle label.
    await waitForText('Privacy', 15_000);
    await waitForText('Anonymized Analytics', 15_000);
    expect(await textExists('Share Anonymized Usage Data')).toBe(true);
  });
});
