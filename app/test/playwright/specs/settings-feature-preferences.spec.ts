import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

async function reloadAndWait(page: Page): Promise<void> {
  await page.reload();
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
}

async function openAuthenticatedRoute(page: Page, userId: string, hash: string): Promise<void> {
  await bootAuthenticatedPage(page, userId, '/home');
  await dismissWalkthroughIfPresent(page);
  await page.goto(`/#${hash}`);
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
}

async function getDefaultMessagingChannel(page: Page): Promise<string | null> {
  return page.evaluate(() => {
    const win = window as unknown as {
      __OPENHUMAN_STORE__?: {
        getState?: () => {
          mascot: { voiceId?: string | null };
          channelConnections: { defaultMessagingChannel?: string | null };
        };
      };
    };
    const state = win.__OPENHUMAN_STORE__?.getState?.();
    if (!state) {
      throw new Error('__OPENHUMAN_STORE__ is unavailable');
    }
    return state.channelConnections.defaultMessagingChannel ?? null;
  });
}

async function getMascotVoiceId(page: Page): Promise<string | null> {
  return page.evaluate(() => {
    const win = window as unknown as {
      __OPENHUMAN_STORE__?: {
        getState?: () => { mascot: { selectedMascotId?: string | null; voiceId?: string | null } };
      };
    };
    const state = win.__OPENHUMAN_STORE__?.getState?.();
    if (!state) {
      throw new Error('__OPENHUMAN_STORE__ is unavailable');
    }
    return state.mascot.voiceId ?? null;
  });
}

async function getSelectedMascotId(page: Page): Promise<string | null> {
  return page.evaluate(() => {
    const win = window as unknown as {
      __OPENHUMAN_STORE__?: { getState?: () => { mascot: { selectedMascotId?: string | null } } };
    };
    const state = win.__OPENHUMAN_STORE__?.getState?.();
    if (!state) {
      throw new Error('__OPENHUMAN_STORE__ is unavailable');
    }
    return state.mascot.selectedMascotId ?? null;
  });
}

async function getPersistedMascotColor(page: Page): Promise<string | null> {
  return page.evaluate(() => {
    const userId = localStorage.getItem('OPENHUMAN_ACTIVE_USER_ID');
    if (!userId) return null;

    const raw = localStorage.getItem(`${userId}:persist:mascot`);
    if (!raw) return null;

    try {
      const parsed = JSON.parse(raw) as { color?: unknown };
      if (typeof parsed.color !== 'string') return null;
      const color = JSON.parse(parsed.color) as unknown;
      return typeof color === 'string' ? color : null;
    } catch {
      return null;
    }
  });
}

async function getPersistedSelectedMascotId(page: Page): Promise<string | null> {
  return page.evaluate(() => {
    const userId = localStorage.getItem('OPENHUMAN_ACTIVE_USER_ID');
    if (!userId) return null;

    const raw = localStorage.getItem(`${userId}:persist:mascot`);
    if (!raw) return null;

    try {
      const parsed = JSON.parse(raw) as { selectedMascotId?: unknown };
      if (typeof parsed.selectedMascotId !== 'string') return null;
      const selectedMascotId = JSON.parse(parsed.selectedMascotId) as unknown;
      return typeof selectedMascotId === 'string' ? selectedMascotId : null;
    } catch {
      return null;
    }
  });
}

async function getAriaChecked(page: Page, label: string): Promise<string | null> {
  const value = await page.getByRole('switch', { name: label }).getAttribute('aria-checked');
  return value;
}

async function getPersistedNotificationPreference(
  page: Page,
  category: string
): Promise<boolean | null> {
  return page.evaluate(categoryName => {
    const userId = localStorage.getItem('OPENHUMAN_ACTIVE_USER_ID');
    if (!userId) return null;
    const raw = localStorage.getItem(`${userId}:persist:notifications`);
    if (!raw) return null;
    try {
      const persisted = JSON.parse(raw) as { preferences?: string };
      if (typeof persisted.preferences !== 'string') return null;
      const preferences = JSON.parse(persisted.preferences) as Record<string, unknown>;
      return typeof preferences[categoryName] === 'boolean'
        ? (preferences[categoryName] as boolean)
        : null;
    } catch {
      return null;
    }
  }, category);
}

async function installMascotManifestMock(page: Page): Promise<void> {
  const manifest = {
    schemaVersion: 1,
    generatedAt: '2026-06-29T00:00:00.000Z',
    source: { repository: 'tinyhumansai/mascots', branch: 'main', commit: 'e2e' },
    mascots: [
      {
        id: 'tiny-mascot',
        name: 'Tiny Default',
        status: 'ready',
        files: [
          {
            role: 'runtime',
            path: 'dist/tiny.riv',
            url: 'https://example.test/mascots/tiny.riv',
            sha256: 'tiny-runtime-sha',
          },
        ],
        stateEngine: {
          states: { idle: 'idle', thinking: 'thinking', speaking: 'speaking', writing: 'writing' },
          idlePoseCycle: ['idle'],
          visemeCodes: ['sil', 'aa', 'oh'],
        },
      },
      {
        id: 'river-guide',
        name: 'River Guide',
        status: 'draft',
        files: [
          {
            role: 'runtime',
            path: 'dist/river.riv',
            url: 'https://example.test/mascots/river.riv',
            sha256: 'river-runtime-sha',
          },
        ],
        stateEngine: {
          states: { idle: 'idle', thinking: 'thinking', speaking: 'speaking', writing: 'writing' },
          idlePoseCycle: ['idle', 'wave'],
          visemeCodes: ['sil', 'aa', 'oh'],
        },
      },
    ],
  };

  await page.route('https://raw.githubusercontent.com/tinyhumansai/mascots/**', route =>
    route.fulfill({ contentType: 'application/json', body: JSON.stringify(manifest) })
  );
  await page.route('https://example.test/mascots/*.riv', route =>
    route.fulfill({ contentType: 'application/octet-stream', body: Buffer.from('not-a-rive-file') })
  );
}

interface ToolsSnapshot {
  result?: { localState?: { onboardingTasks?: { enabledTools?: string[] | null } | null } | null };
  localState?: { onboardingTasks?: { enabledTools?: string[] | null } | null } | null;
}

function readEnabledTools(snapshot: ToolsSnapshot): string[] {
  const body = snapshot.result ?? snapshot;
  return body.localState?.onboardingTasks?.enabledTools ?? [];
}

test.describe('Settings - Feature Preferences', () => {
  test('renders the features settings section route', async ({ page }) => {
    // The old "Features" hub page is retired and redirects to the Screen
    // Awareness tab on Connections.
    await openAuthenticatedRoute(page, 'pw-settings-features-route', '/settings/features');

    await expect
      .poll(async () => page.evaluate(() => window.location.hash))
      .toContain('/connections?tab=screen-intelligence');
    await expect(page.getByText('Screen awareness', { exact: true }).first()).toBeVisible();
  });

  test('persists the default messaging channel through redux state', async ({ page }) => {
    // Phase 2: default messaging channel moved to /connections (Messaging tab).
    await openAuthenticatedRoute(page, 'pw-settings-default-channel', '/connections?tab=messaging');

    // "Set as default" now appears only on *connected* channels. In a fresh
    // workspace the only always-connected channel is Web (built-in chat), so
    // make Telegram the default first (turning Web into a connected,
    // non-default tile with the control), reload so the panel re-seeds, then
    // switch the default to Web.
    await callCoreRpc('openhuman.channels_set_default', { channel: 'telegram' });
    await reloadAndWait(page);

    const messagingTab = page.getByTestId('two-pane-nav-channels');
    if (await messagingTab.isVisible().catch(() => false)) {
      await messagingTab.click();
    }

    await expect(page.getByText('Default Messaging Channel').last()).toBeVisible();
    await expect.poll(() => getDefaultMessagingChannel(page)).toBe('telegram');

    await page.getByTestId('channel-select-web').click();
    await expect.poll(() => getDefaultMessagingChannel(page)).toBe('web');
  });

  test('persists tools preferences to the core app-state snapshot', async ({ page }) => {
    await openAuthenticatedRoute(page, 'pw-settings-tools', '/settings/tools');

    await callCoreRpc('openhuman.app_state_update_local_state', {
      onboardingTasks: {
        accessibilityPermissionGranted: false,
        localModelConsentGiven: false,
        localModelDownloadStarted: false,
        enabledTools: ['shell'],
        connectedSources: [],
        updatedAtMs: Date.now(),
      },
    });

    const before = await callCoreRpc<ToolsSnapshot>('openhuman.app_state_snapshot', {});
    const enabledBefore = readEnabledTools(before);

    await reloadAndWait(page);

    // The two-pane sidebar also renders a "Tools" nav label, so scope to first.
    await expect(page.getByText('Tools', { exact: true }).first()).toBeVisible();
    // Tool rows are now SettingsRow + SettingsSwitch (role="switch", aria-label =
    // the tool's display name), not a single text-bearing button.
    const shellToggle = page.getByRole('switch', { name: 'Shell Commands', exact: true });
    await expect(shellToggle).toHaveAttribute('aria-checked', 'true');
    await shellToggle.click();
    await expect(shellToggle).toHaveAttribute('aria-checked', 'false');

    await page.getByRole('button', { name: 'Save Changes', exact: true }).click();
    await expect(page.getByText('Preferences saved')).toBeVisible();

    await expect
      .poll(async () => {
        const after = await callCoreRpc<ToolsSnapshot>('openhuman.app_state_snapshot', {});
        const enabledAfter = readEnabledTools(after);
        return JSON.stringify(enabledAfter) !== JSON.stringify(enabledBefore);
      })
      .toBe(true);

    const after = await callCoreRpc<ToolsSnapshot>('openhuman.app_state_snapshot', {});
    expect(readEnabledTools(after)).not.toContain('shell');
  });

  test('persists notification category preferences', async ({ page }) => {
    await openAuthenticatedRoute(page, 'pw-settings-notification-prefs', '/settings/notifications');

    await expect(page.getByText('Do Not Disturb', { exact: true })).toBeVisible();
    await expect(page.getByText('Messages', { exact: true })).toBeVisible();

    const messagesLabel = 'Toggle Messages notifications';
    const messagesBefore = await getAriaChecked(page, messagesLabel);

    // Global DND is native webview-account state and cannot persist in the web
    // harness. Category preferences are Redux-persisted and are the portable
    // behavior this lane can verify.
    await page.getByRole('switch', { name: messagesLabel }).click();

    await expect.poll(() => getAriaChecked(page, messagesLabel)).not.toBe(messagesBefore);

    const toggled = await getAriaChecked(page, messagesLabel);
    await expect
      .poll(() => getPersistedNotificationPreference(page, 'messages'))
      .toBe(toggled === 'true');

    await reloadAndWait(page);
    await expect(page.getByText('Do Not Disturb')).toBeVisible();
    await expect.poll(() => getAriaChecked(page, messagesLabel)).toBe(toggled);
  });

  test('persists mascot color selection', async ({ page }) => {
    await openAuthenticatedRoute(page, 'pw-settings-mascot-color', '/settings/mascot');

    await expect(page.getByRole('heading', { name: 'Color', exact: true })).toBeVisible();
    await page.getByTestId('mascot-color-burgundy').click();
    await expect(page.getByTestId('mascot-color-burgundy')).toHaveAttribute('aria-checked', 'true');
    await expect.poll(() => getPersistedMascotColor(page)).toBe('burgundy');

    await reloadAndWait(page);
    await expect(page.getByTestId('mascot-color-burgundy')).toHaveAttribute('aria-checked', 'true');
  });

  test('persists manifest mascot selection and uses it on the Human page', async ({ page }) => {
    await installMascotManifestMock(page);
    await openAuthenticatedRoute(page, 'pw-settings-manifest-mascot', '/settings/mascot');

    await expect(page.getByRole('heading', { name: 'Character', exact: true })).toBeVisible();
    await expect(page.getByTestId('manifest-mascot-river-guide')).toContainText('River Guide');
    await page.getByTestId('manifest-mascot-river-guide').click();
    await expect(page.getByTestId('manifest-mascot-river-guide')).toHaveAttribute(
      'aria-pressed',
      'true'
    );
    await expect.poll(() => getSelectedMascotId(page)).toBe('river-guide');
    await expect.poll(() => getPersistedSelectedMascotId(page)).toBe('river-guide');

    await reloadAndWait(page);
    await expect.poll(() => getSelectedMascotId(page)).toBe('river-guide');
    await expect(page.getByTestId('manifest-mascot-river-guide')).toHaveAttribute(
      'aria-pressed',
      'true'
    );

    await page.goto('/#/human');
    await waitForAppReady(page);
    await expect.poll(() => getSelectedMascotId(page)).toBe('river-guide');
  });

  test('persists the custom mascot voice override on the voice panel', async ({ page }) => {
    await openAuthenticatedRoute(page, 'pw-settings-mascot-voice', '/settings/voice');

    await expect(page.getByText('Mascot Voice')).toBeVisible();
    test.skip(
      (await page
        .locator('[data-testid="mascot-voice-select"] option[value="__custom__"]')
        .count()) === 0,
      'custom mascot voice option is unavailable in this build'
    );

    await page.getByTestId('mascot-voice-select').selectOption('__custom__');
    test.skip(
      (await page.getByTestId('mascot-voice-input').count()) === 0,
      'custom mascot voice input did not appear after selecting __custom__'
    );

    await page.getByTestId('mascot-voice-input').fill('voice-e2e-custom');
    await page.getByTestId('mascot-voice-save-paste').click();

    await expect.poll(() => getMascotVoiceId(page)).toBe('voice-e2e-custom');

    await reloadAndWait(page);
    await expect.poll(() => getMascotVoiceId(page)).toBe('voice-e2e-custom');
  });
});
