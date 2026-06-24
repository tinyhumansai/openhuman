import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;

async function mockFetch(path: string, init?: RequestInit): Promise<unknown> {
  const response = await fetch(`${MOCK_BASE}${path}`, init);
  if (!response.ok) throw new Error(`mock ${path} failed: ${response.status}`);
  return response.json();
}

async function setMockBehavior(behavior: Record<string, unknown>): Promise<void> {
  await mockFetch('/__admin/behavior', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ behavior }),
  });
}

test.describe('Top-level functional flows', () => {
  test('workflows create and delete round-trip through the top-level page', async ({ page }) => {
    // Use a slug-safe name: create stores `name: <slug>` (the display string is
    // slugified), and the runner heading / workflows_list both surface that
    // slug — so keep the typed name already slug-shaped to assert against it.
    const name = `pw-workflow-${Date.now()}`;
    await bootAuthenticatedPage(page, 'pw-workflows-create-delete', '/workflows');
    await dismissWalkthroughIfPresent(page);

    await page.getByTestId('workflows-create-btn').click();
    await expect(page.getByRole('dialog', { name: /New Workflow/i })).toBeVisible();
    await page.getByLabel(/Name/).fill(name);
    await page.getByLabel(/Description/).fill('Created by Playwright to cover workflow UX.');
    await page.getByLabel(/When to use/).fill('Use when validating E2E workflow CRUD.');
    await page.getByRole('button', { name: 'Create workflow' }).click();

    await expect(page.getByRole('heading', { name })).toBeVisible({ timeout: 15_000 });
    // The display name is already slug-shaped, so the workflow id === name
    // (create derives the id by slugifying the name). Use it directly for the
    // delete selectors instead of round-tripping through workflows_list.
    const id = name;

    // Creating navigates to the runner page locked to the new workflow, so
    // return to the Workflows list to delete it. The delete action lives in
    // the card's "More actions" menu.
    await page.goto('/#/workflows');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);

    const card = page.getByTestId(`workflow-card-${id}`);
    await card.getByTitle('More actions').click();
    await page.getByTestId(`workflow-uninstall-${id}`).click();
    // Workflows/automations now render inside the Settings modal (also
    // role="dialog"), so target the uninstall confirm control specifically
    // rather than a generic getByRole('dialog') that would match both.
    await expect(page.getByTestId('uninstall-skill-confirm')).toBeVisible();
    await page.getByTestId('uninstall-skill-confirm').click();
    await expect(page.getByText(name)).toHaveCount(0, { timeout: 15_000 });
  });

  test('invites copies a rendered invite code and redeems a typed code', async ({ page }) => {
    const inviteCode = `PW${Date.now().toString().slice(-6)}`;
    await setMockBehavior({
      inviteCodes: JSON.stringify([
        { _id: 'invite-pw-1', code: inviteCode, currentUses: 0, maxUses: 1, usageHistory: [] },
      ]),
    });

    await page.addInitScript(() => {
      Object.defineProperty(navigator, 'clipboard', {
        configurable: true,
        value: {
          writeText: async (text: string) => {
            window.localStorage.setItem('pw:last-copied-invite', text);
          },
        },
      });
    });
    await bootAuthenticatedPage(page, 'pw-invites-copy-redeem', '/invites');
    await dismissWalkthroughIfPresent(page);

    await expect(page.getByText(inviteCode)).toBeVisible();
    await page.getByTitle('Copy').click();
    await expect
      .poll(() => page.evaluate(() => localStorage.getItem('pw:last-copied-invite')))
      .toBe(inviteCode);

    await page.getByPlaceholder('Search').fill('welcome42');
    await page.getByRole('button', { name: 'Referrals' }).click();
    await expect(page.getByText('Success')).toBeVisible({ timeout: 15_000 });
  });

  test('major top-level pages render actionable UI without blanking', async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-top-level-ui', '/home');
    const routes: Array<[string, RegExp]> = [
      // Home folded into the unified chat surface — /home redirects to /chat.
      ['/home', /New Conversation|Threads/],
      ['/connections', /Composio Integrations|Composio|Channels|MCP Servers/],
      ['/chat', /New Conversation|No messages yet|Threads/],
      ['/settings/notifications-hub', /Notifications/],
      ['/notifications', /Notifications|System Events/],
      ['/rewards', /Rewards|Referrals|Redeem/],
    ];

    for (const [hash, text] of routes) {
      await page.goto(`/#${hash}`);
      await waitForAppReady(page);
      await dismissWalkthroughIfPresent(page);
      await expect(page.locator('#root')).toContainText(text);
    }
  });
});
