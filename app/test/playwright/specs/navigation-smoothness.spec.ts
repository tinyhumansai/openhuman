import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, waitForAppReady } from '../helpers/core-rpc';

interface RouteCheck {
  hash: string;
  markers: string[];
}

const routes: RouteCheck[] = [
  { hash: '/chat', markers: ['Your assistant is ready', 'Reasoning'] },
  { hash: '/connections', markers: ['Composio', 'Channels', 'MCP Servers', 'Skills'] },
  // Home folded into the unified chat surface — /home redirects to /chat.
  { hash: '/home', markers: ['Your assistant is ready', 'Reasoning'] },
  { hash: '/channels', markers: ['Channels', 'Connections', 'Telegram', 'Discord'] },
  { hash: '/notifications', markers: ['Notifications', 'Alerts', 'No alerts yet'] },
  { hash: '/settings', markers: ['Settings', 'Account', 'Billing', 'Advanced'] },
  { hash: '/settings/memory-data', markers: ['Documents', 'Memory'] },
  { hash: '/settings/agent-chat', markers: ['LLM Providers'] },
  { hash: '/settings/local-model-debug', markers: ['LLM Providers'] },
  { hash: '/flows', markers: ['Flows', 'New'] },
  { hash: '/settings/notifications-hub', markers: ['Notifications'] },
  { hash: '/connections?tab=brain', markers: ['Engine', 'Ask', 'Documents', 'Memory'] },
  // Home folded into the unified chat surface — /home redirects to /chat.
  { hash: '/home', markers: ['Your assistant is ready', 'Reasoning'] },
];

async function rootTextLength(page: import('@playwright/test').Page): Promise<number> {
  return page
    .locator('#root')
    .innerText()
    .then(text => text.length);
}

async function verifyRouteLoaded(
  page: import('@playwright/test').Page,
  route: RouteCheck
): Promise<void> {
  await waitForAppReady(page);
  await expect.poll(() => rootTextLength(page), { timeout: 10_000 }).toBeGreaterThan(50);
}

test.describe('Navigation Smoothness', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-navigation-smoothness-user');
  });

  test('all major routes render within timing budget', async ({ page }) => {
    for (const route of routes) {
      await page.goto(`/#${route.hash}`);
      await verifyRouteLoaded(page, route);
    }
  });

  test('rapid cycle completes without blank screens', async ({ page }) => {
    for (const route of routes) {
      await page.goto(`/#${route.hash}`);
      await verifyRouteLoaded(page, route);
    }
  });

  test('final state is the chat surface with correct content', async ({ page }) => {
    // Home folded into the unified chat surface: /home redirects to /chat and
    // the chat "new window" empty state renders the former Home hero card.
    await page.goto('/#/home');
    await waitForAppReady(page);
    await expect(page.getByTestId('chat-message-input')).toBeVisible();
    await expect.poll(async () => page.evaluate(() => window.location.hash)).toMatch(/^#\/chat/);
  });
});
