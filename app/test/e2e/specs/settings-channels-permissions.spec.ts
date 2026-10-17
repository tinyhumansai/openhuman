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
import { callOpenhumanRpc } from '../helpers/core-rpc';
import {
  clickSelector,
  getSwitchCheckedByLabel,
  textExists,
  toggleSwitchByLabel,
  visibleTextExists,
  waitForText,
} from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

/** Read the persisted default messaging channel from the renderer's redux store. */
async function defaultMessagingChannel(): Promise<string | null> {
  return browser.execute(() => {
    const win = window as unknown as {
      __OPENHUMAN_STORE__?: {
        getState?: () => { channelConnections?: { defaultMessagingChannel?: string | null } };
      };
    };
    return (
      win.__OPENHUMAN_STORE__?.getState?.().channelConnections?.defaultMessagingChannel ?? null
    );
  });
}

const USER_ID = 'e2e-settings-channels';

describe('Settings - Channels & Permissions', () => {
  before(async function () {
    // resetApp bring-up can run ~25-30s and race the default 30s Mocha hook
    // budget; raise it.
    this.timeout(90_000);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('allows switching default messaging channel (13.2.1)', async () => {
    // resetApp must finish dismissing the post-onboarding Joyride before a
    // settings flow starts; otherwise the spotlight overlay intercepts the
    // hash navigation and leaves the app on Chat.
    expect(await visibleTextExists('Skip tour')).toBe(false);

    // The messaging panel now offers "Set as default" only on *connected*
    // channels. In a fresh workspace the only always-connected channel is Web
    // (the built-in chat), so we make Telegram the default first — that turns
    // Web into a connected, non-default tile that exposes the control — then
    // switch the default to Web through the UI.
    await callOpenhumanRpc('openhuman.channels_set_default', { channel: 'telegram' });

    // Navigate away and back so the messaging panel re-seeds the default from
    // the core (it reads the persisted default when the page mounts).
    await navigateViaHash('/home');
    await navigateViaHash('/connections?tab=messaging');

    await waitForText('Web', 15_000);
    expect(await textExists('Telegram')).toBe(true);
    expect(await textExists('Web')).toBe(true);

    // Confirm the panel seeded Telegram as the default before we switch.
    await browser.waitUntil(async () => (await defaultMessagingChannel()) === 'telegram', {
      timeout: 10_000,
      interval: 500,
      timeoutMsg: 'messaging panel did not seed Telegram as the default',
    });

    // Switch to Web via the stable channel-select test id (its "Set as default"
    // control is present because Web is always connected).
    await clickSelector('[data-testid="channel-select-web"]');
    await browser.waitUntil(async () => (await defaultMessagingChannel()) === 'web', {
      timeout: 10_000,
      interval: 500,
      timeoutMsg: 'default messaging channel did not switch to web',
    });
  });

  it('renders privacy settings and analytics toggle (13.2.2)', async () => {
    await navigateViaHash('/settings/privacy');

    await waitForText('Privacy', 15_000);
    // PrivacyPanel's analytics section was renamed: t('privacy.anonymizedAnalytics')
    // is now "Product Analytics" and the toggle label t('privacy.shareAnonymizedData')
    // is "Share Product Analytics and Diagnostics".
    await waitForText('Product Analytics', 15_000);
    expect(await textExists('Share Product Analytics and Diagnostics')).toBe(true);

    const analyticsLabel = 'Share Product Analytics and Diagnostics';
    expect(await textExists(analyticsLabel)).toBe(true);
    const initialState = await getSwitchCheckedByLabel('privacy-analytics-toggle', analyticsLabel);
    await toggleSwitchByLabel('privacy-analytics-toggle', analyticsLabel);
    const expectedState = !initialState;
    await browser.waitUntil(
      async () =>
        (await getSwitchCheckedByLabel('privacy-analytics-toggle', analyticsLabel)) ===
        expectedState,
      { timeout: 10_000, timeoutMsg: 'analytics preference did not change' }
    );

    // Leaving and reopening the route remounts PrivacyPanel from the refreshed
    // core snapshot, proving the preference was persisted by the core.
    await navigateViaHash('/settings/account');
    await navigateViaHash('/settings/privacy');
    await waitForText(analyticsLabel, 15_000);
    await browser.waitUntil(
      async () =>
        (await getSwitchCheckedByLabel('privacy-analytics-toggle', analyticsLabel)) ===
        expectedState,
      {
        timeout: 10_000,
        timeoutMsg: 'analytics preference was lost after reopening privacy settings',
      }
    );
  });
});
