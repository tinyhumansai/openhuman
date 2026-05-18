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
  before(async () => {
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('allows switching default messaging channel (13.2.1)', async () => {
    await navigateViaHash('/settings/messaging');

    await waitForText('Default Messaging Channel', 15_000);
    expect(await textExists('Telegram')).toBe(true);
    expect(await textExists('Discord')).toBe(true);

    await clickText('Discord');
    await waitForText('Active route: discord via', 5_000);
  });

  it('renders privacy settings and analytics toggle (13.2.2)', async () => {
    await navigateViaHash('/settings/privacy');

    await waitForText('Privacy', 15_000);
    await waitForText('Data Sharing', 15_000);
    expect(await textExists('Share Anonymized Usage Data')).toBe(true);
    await waitForText('Permission Metadata', 5_000);
  });
});
