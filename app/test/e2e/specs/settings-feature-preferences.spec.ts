// @ts-nocheck
import { browser, expect } from '@wdio/globals';

import { waitForApp } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import {
  clickSelector,
  clickText,
  setSelectValueByTestId,
  waitForText,
} from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-settings-feature-preferences';

async function reloadAndReturnTo(route: string, markerText: string): Promise<void> {
  await browser.execute(() => window.location.reload());
  await browser.pause(3000);
  await navigateViaHash(route);
  await waitForText(markerText, 15_000);
}

async function switchState(ariaLabel: string): Promise<string | null> {
  return await browser.execute(label => {
    const el = document.querySelector<HTMLElement>(`button[aria-label="${label}"]`);
    return el?.getAttribute('aria-checked') ?? null;
  }, ariaLabel);
}

async function mascotColorChecked(colorId: string): Promise<string | null> {
  return await browser.execute(id => {
    const el = document.querySelector<HTMLElement>(`[data-testid="mascot-color-${id}"]`);
    return el?.getAttribute('aria-checked') ?? null;
  }, colorId);
}

async function mascotVoiceIdFromStore(): Promise<string | null> {
  return await browser.execute(() => {
    const win = window as unknown as {
      __OPENHUMAN_STORE__?: { getState?: () => { mascot?: { voiceId?: string | null } } };
    };
    return win.__OPENHUMAN_STORE__?.getState?.().mascot?.voiceId ?? null;
  });
}

async function mascotVoiceIdFromPersistedBlob(): Promise<string | null> {
  return await browser.execute(() => {
    const activeUserId = window.localStorage.getItem('OPENHUMAN_ACTIVE_USER_ID');
    if (!activeUserId) return null;
    const raw = window.localStorage.getItem(`${activeUserId}:persist:mascot`);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Record<string, string>;
    const voiceIdRaw = parsed.voiceId;
    if (!voiceIdRaw) return null;
    return JSON.parse(voiceIdRaw) as string | null;
  });
}

async function defaultMessagingChannelFromStore(): Promise<string | null> {
  return await browser.execute(() => {
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

describe('Settings - Feature Preferences', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('renders the features settings section route', async () => {
    await navigateViaHash('/settings/features');

    await waitForText('Features', 15_000);
    await waitForText('Screen Awareness', 15_000);
    await waitForText('Messaging Channels', 15_000);
    await waitForText('Notifications', 15_000);
    await waitForText('Tools', 15_000);
  });

  it('persists the default messaging channel through redux state', async () => {
    await navigateViaHash('/settings/messaging');

    await waitForText('Default Messaging Channel', 15_000);
    await clickText('Discord', 10_000);
    await browser.waitUntil(async () => (await defaultMessagingChannelFromStore()) === 'discord', {
      timeout: 10_000,
      interval: 500,
      timeoutMsg: 'default channel did not update',
    });
  });

  it('persists tools preferences to the core app-state snapshot', async () => {
    const before = await callOpenhumanRpc('openhuman.app_state_snapshot', {});
    expect(before.ok).toBe(true);
    const enabledBefore = before.result?.result?.localState?.onboardingTasks?.enabledTools ?? [];

    await navigateViaHash('/settings/tools');
    await waitForText('Tools', 15_000);

    expect(await clickText('Shell Commands', 10_000)).toBeDefined();
    await clickText('Save Changes', 10_000);
    await waitForText('Preferences saved', 10_000);

    await browser.waitUntil(
      async () => {
        const after = await callOpenhumanRpc('openhuman.app_state_snapshot', {});
        const enabledAfter = after.result?.result?.localState?.onboardingTasks?.enabledTools ?? [];
        return JSON.stringify(enabledAfter) !== JSON.stringify(enabledBefore);
      },
      { timeout: 15_000, interval: 500, timeoutMsg: 'tools settings did not persist' }
    );
  });

  it('persists notifications DND and category preferences', async () => {
    await navigateViaHash('/settings/notifications');

    await waitForText('Do Not Disturb', 15_000);
    await waitForText('Messages', 15_000);

    expect(await clickSelector('button[aria-label="Toggle Do Not Disturb"]')).toBe(true);
    expect(await clickSelector('button[aria-label="Toggle Messages notifications"]')).toBe(true);
    await browser.pause(1000);
    await reloadAndReturnTo('/settings/notifications', 'Do Not Disturb');

    expect(await switchState('Toggle Do Not Disturb')).toBe('true');
    expect(await switchState('Toggle Messages notifications')).toBe('false');
  });

  it('persists mascot color selection', async () => {
    await navigateViaHash('/settings/mascot');

    await waitForText('Color', 15_000);
    expect(await clickSelector('[data-testid="mascot-color-burgundy"]')).toBe(true);
    await browser.pause(1000);
    await reloadAndReturnTo('/settings/mascot', 'Color');

    expect(await mascotColorChecked('burgundy')).toBe('true');
  });

  it('persists the custom mascot voice override on the voice panel', async () => {
    await navigateViaHash('/settings/voice');

    await waitForText('Mascot Voice', 20_000);
    expect(await setSelectValueByTestId('mascot-voice-select', '__custom__')).toBe(true);
    const customVoiceInput = await browser.$('[data-testid="mascot-voice-input"]');
    await customVoiceInput.waitForExist({ timeout: 10_000 });
    await customVoiceInput.setValue('voice-e2e-custom');
    expect(await clickSelector('[data-testid="mascot-voice-save-paste"]')).toBe(true);
    await browser.waitUntil(async () => (await mascotVoiceIdFromStore()) === 'voice-e2e-custom', {
      timeout: 10_000,
      interval: 500,
      timeoutMsg: 'custom mascot voice did not update',
    });
    await browser.waitUntil(
      async () => (await mascotVoiceIdFromPersistedBlob()) === 'voice-e2e-custom',
      {
        timeout: 15_000,
        interval: 500,
        timeoutMsg: 'custom mascot voice did not persist to storage',
      }
    );
    await reloadAndReturnTo('/settings/voice', 'Mascot Voice');

    await browser.waitUntil(async () => (await mascotVoiceIdFromStore()) === 'voice-e2e-custom', {
      timeout: 15_000,
      interval: 500,
      timeoutMsg: 'custom mascot voice did not persist',
    });
  });
});
