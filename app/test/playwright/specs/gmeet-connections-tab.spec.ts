import { expect, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

test.describe('Google Meet Connections tab', () => {
  test.beforeEach(async ({ page }) => {
    // Phase 2: /skills → /connections; Phase 2b: MeetingBotsCard moved from
    // Tools tab to the new Talents tab (?tab=talents). Back-compat: ?tab=meetings
    // alias still works and resolves to talents.
    await bootAuthenticatedPage(page, 'pw-gmeet-connections-tab-user', '/connections?tab=talents');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);
  });

  test('opens the Talents tab and shows the meeting link modal', async ({ page }) => {
    await expect
      .poll(async () => page.evaluate(() => window.location.hash), { timeout: 10_000 })
      .toContain('/connections');

    // Phase 2b: MeetingBotsCard moved to the Talents tab; there is no longer a
    // "Google Meet" sub-tab.  The PillTabBar "Talents" pill carries the
    // aria-selected=true attribute when the Talents surface is active.
    await expect(page.getByRole('tab', { name: 'Talents', exact: true })).toHaveAttribute(
      'aria-selected',
      'true'
    );

    await page.getByTestId('meeting-bots-banner').click();

    // PASSIVE MODE: the modal asks for the Meeting link only. The
    // "Your Name in This Meeting" (respondTo) text field added in #3555
    // is hidden because the backend bot no longer listens for a wake
    // phrase. It stays Google-Meet only — no other platforms.
    const dialog = page.getByRole('dialog', { name: 'Send OpenHuman to a meeting' });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByLabel('Meeting link')).toBeVisible();
    await expect(dialog.locator('input[type="url"]')).toHaveCount(1);
    await expect(dialog.locator('input[type="text"]')).toHaveCount(0);
    await expect(dialog.getByText('Zoom')).toHaveCount(0);
    await expect(dialog.getByText('Microsoft Teams')).toHaveCount(0);
  });
});
