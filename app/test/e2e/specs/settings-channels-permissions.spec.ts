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
import { clickSelector, textExists, waitForText } from '../helpers/element-helpers';
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
    // Phase 2: Default Messaging Channel UI is at /connections (Messaging tab).
    // Old /skills?tab=channels → /connections?tab=messaging.
    await navigateViaHash('/connections?tab=messaging');

    await waitForText('Default Messaging Channel', 15_000);
    expect(await textExists('Telegram')).toBe(true);
    expect(await textExists('Discord')).toBe(true);

    // Select via the stable channel-select test id rather than the ambiguous
    // "Discord" text (which also appears on connection tiles / help copy).
    await clickSelector('[data-testid="channel-select-discord"]');
    // Confirm the selection persisted to redux state (the Connections messaging
    // tab no longer renders the legacy "Active route" line).
    await browser.waitUntil(
      async () =>
        (await browser.execute(() => {
          const win = window as unknown as {
            __OPENHUMAN_STORE__?: {
              getState?: () => { channelConnections?: { defaultMessagingChannel?: string | null } };
            };
          };
          return win.__OPENHUMAN_STORE__?.getState?.().channelConnections?.defaultMessagingChannel;
        })) === 'discord',
      {
        timeout: 10_000,
        interval: 500,
        timeoutMsg: 'default messaging channel did not switch to discord',
      }
    );
  });

  it('renders privacy settings and analytics toggle (13.2.2)', async () => {
    await navigateViaHash('/settings/privacy');

    await waitForText('Privacy', 15_000);
    // PrivacyPanel's analytics section was renamed: t('privacy.anonymizedAnalytics')
    // is now "Product Analytics" and the toggle label t('privacy.shareAnonymizedData')
    // is "Share Product Analytics and Diagnostics".
    await waitForText('Product Analytics', 15_000);
    expect(await textExists('Share Product Analytics and Diagnostics')).toBe(true);
    // Capability list section is "What leaves your computer" (not "Permission Metadata")
    await waitForText('What leaves your computer', 5_000);
  });
});
