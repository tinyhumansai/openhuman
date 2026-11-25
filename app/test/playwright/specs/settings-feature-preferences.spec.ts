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
  test('falls through the retired features settings route to settings home', async ({ page }) => {
    await openAuthenticatedRoute(page, 'pw-settings-features-route', '/settings/features');
    for (const route of [
      '/settings/features',
      '/settings/screen-intelligence',
      '/settings/screen-awareness-debug',
    ]) {
      await page.goto(`/#${route}`);
      await waitForAppReady(page);
      await expect
        .poll(async () => page.evaluate(() => window.location.hash))
        .toMatch(/^#\/settings(?:\/account)?$/);
      await expect(page.getByText('Screen Intelligence', { exact: true })).toHaveCount(0);
      await expect(page.getByText('Screen Awareness Debug', { exact: true })).toHaveCount(0);
    }
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

    await expect(page.getByTestId('channel-select-telegram')).toBeVisible();
    await expect.poll(() => getDefaultMessagingChannel(page)).toBe('telegram');

    await page.getByTestId('channel-select-web').click();
    await expect.poll(() => getDefaultMessagingChannel(page)).toBe('web');
  });

  test('persists tools preferences to the core app-state snapshot', async ({ page }) => {
    await openAuthenticatedRoute(page, 'pw-settings-tools', '/settings/tools');

    await callCoreRpc('openhuman.app_state_update_local_state', {
      onboardingTasks: {
        accessibilityPermissionGranted: false,
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
    await openAuthenticatedRoute(page, 'pw-settings-mascot-voice', '/settings/face');

    await expect(page.getByRole('heading', { name: 'Voice', exact: true })).toBeVisible();
    const customVoiceOption = page.locator(
      '[data-testid="mascot-voice-select"] option[value="__custom__"]'
    );
    await expect(customVoiceOption).toHaveCount(1);

    await page.getByTestId('mascot-voice-select').selectOption('__custom__');
    await expect(page.getByTestId('mascot-voice-input')).toBeVisible();

    await page.getByTestId('mascot-voice-input').fill('voice-e2e-custom');
    await page.getByTestId('mascot-voice-save-paste').click();

    await expect.poll(() => getMascotVoiceId(page)).toBe('voice-e2e-custom');

    await reloadAndWait(page);
    await expect.poll(() => getMascotVoiceId(page)).toBe('voice-e2e-custom');
  });
});
