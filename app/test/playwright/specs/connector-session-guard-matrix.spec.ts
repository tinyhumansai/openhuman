import { expect, type Page, test } from '@playwright/test';

import {
  bootRuntimeReadyGuestPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  signInViaCallbackToken,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;

type Toolkit = {
  slug: 'discord' | 'github' | 'jira';
  name: string;
  action: string;
  connectionId: string;
};

const TOOLKITS: Toolkit[] = [
  {
    slug: 'discord',
    name: 'Discord',
    action: 'DISCORD_FETCH_MESSAGES',
    connectionId: 'c-discord-pw',
  },
  { slug: 'github', name: 'GitHub', action: 'GITHUB_LIST_REPOS', connectionId: 'c-github-pw' },
  { slug: 'jira', name: 'Jira', action: 'JIRA_SEARCH_ISSUES', connectionId: 'c-jira-pw' },
];

async function mockFetch(path: string, init?: RequestInit): Promise<{ data?: unknown }> {
  const response = await fetch(`${MOCK_BASE}${path}`, init);
  if (!response.ok) throw new Error(`mock ${path} failed: ${response.status}`);
  return response.json() as Promise<{ data?: unknown }>;
}

async function resetMock(): Promise<void> {
  await mockFetch('/__admin/reset', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ keepBehavior: false, keepRequests: false }),
  });
}

async function setMockBehavior(behavior: Record<string, unknown>): Promise<void> {
  await mockFetch('/__admin/behavior', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ behavior }),
  });
}

async function getRequestLog(): Promise<Array<{ method?: string; url?: string; body?: string }>> {
  const payload = await mockFetch('/__admin/requests');
  return (payload.data as Array<{ method?: string; url?: string; body?: string }>) ?? [];
}

async function seedToolkits(status: 'ACTIVE' | 'FAILED' | 'EXPIRED' = 'ACTIVE'): Promise<void> {
  await setMockBehavior({
    composioToolkits: JSON.stringify(TOOLKITS.map(toolkit => toolkit.slug)),
    composioConnections: JSON.stringify(
      TOOLKITS.map(toolkit => ({ id: toolkit.connectionId, toolkit: toolkit.slug, status }))
    ),
  });
}

async function bootSkills(page: Page, userId: string): Promise<void> {
  await resetMock();
  await seedToolkits('ACTIVE');
  await bootRuntimeReadyGuestPage(page);
  await signInViaCallbackToken(page, userId);
  // Phase 2: /skills → /connections, "Composio" tab renamed to "Apps"
  await page.goto('/#/connections');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  await page.getByRole('tab', { name: 'Composio' }).click();
  await expect(
    page.getByRole('heading', { name: 'Composio Integrations', exact: true })
  ).toBeVisible({ timeout: 20_000 });
}

async function assertSessionAlive(page: Page): Promise<void> {
  // Phase 2: /skills → /connections
  await expect
    .poll(async () =>
      page.evaluate(() => {
        const snapshot = (
          window as unknown as {
            __OPENHUMAN_CORE_STATE__?: () => {
              snapshot?: {
                currentUser?: { _id?: string | null } | null;
                sessionToken?: string | null;
              };
            };
          }
        ).__OPENHUMAN_CORE_STATE__?.()?.snapshot;
        return {
          // The connections page now appends an active-tab query (e.g.
          // `#/connections?tab=composio`); strip it so we assert we're still on
          // the connections route (session not nuked), not the exact sub-tab.
          hash: window.location.hash.replace(/\?.*$/, ''),
          hasUser: Boolean(snapshot?.currentUser?._id),
          hasToken: Boolean(snapshot?.sessionToken),
        };
      })
    )
    .toEqual({ hash: '#/connections', hasUser: true, hasToken: true });
}

test.describe('Connector session guard matrix', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const slug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootSkills(page, `pw-connector-matrix-${slug}`);
  });

  for (const toolkit of TOOLKITS) {
    test(`${toolkit.name} card opens management UI and authorize routes through mock backend`, async ({
      page,
    }) => {
      const card = page.getByTestId(`skill-install-composio-${toolkit.slug}`);
      await expect(card).toContainText(toolkit.name);
      await card.click();
      await expect(page.getByRole('dialog', { name: new RegExp(toolkit.name, 'i') })).toBeVisible();
      await page.keyboard.press('Escape');

      await callCoreRpc('openhuman.composio_authorize', { toolkit: toolkit.slug });
      const requests = await getRequestLog();
      const auth = requests.find(
        request =>
          request.method === 'POST' &&
          request.url?.includes('/agent-integrations/composio/authorize') &&
          JSON.parse(request.body || '{}').toolkit === toolkit.slug
      );
      expect(auth).toBeDefined();
      await assertSessionAlive(page);
    });
  }

  test('Jira connect modal exposes the required site/subdomain input', async ({ page }) => {
    await setMockBehavior({
      composioConnections: JSON.stringify(
        TOOLKITS.filter(toolkit => toolkit.slug !== 'jira').map(toolkit => ({
          id: toolkit.connectionId,
          toolkit: toolkit.slug,
          status: 'ACTIVE',
        }))
      ),
    });
    await page.reload();
    await waitForAppReady(page);
    // Phase 2: "Composio" tab renamed to "Apps"
    await page.getByRole('tab', { name: 'Composio' }).click();
    await page.getByTestId('skill-install-composio-jira').click();
    const dialog = page.getByRole('dialog', { name: /Jira/i });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByRole('textbox', { name: /Atlassian subdomain/i })).toBeVisible();
  });

  test('failed and expired connector states keep the user signed in and page usable', async ({
    page,
  }) => {
    await seedToolkits('FAILED');
    await page.reload();
    await waitForAppReady(page);
    // Phase 2: "Composio" tab renamed to "Apps"
    await page.getByRole('tab', { name: 'Composio' }).click();
    await expect(page.getByTestId('skill-install-composio-discord')).toContainText('Discord');
    await assertSessionAlive(page);

    await seedToolkits('EXPIRED');
    await page.reload();
    await waitForAppReady(page);
    // Phase 2: "Composio" tab renamed to "Apps"
    await page.getByRole('tab', { name: 'Composio' }).click();
    await expect(page.getByTestId('skill-install-composio-github')).toContainText(
      /Reconnect|GitHub/
    );
    await assertSessionAlive(page);
  });

  test('Composio execute and disconnect errors do not clear auth session', async ({ page }) => {
    await setMockBehavior({ composioExecuteFails: '500' });
    await expect(
      callCoreRpc('openhuman.composio_execute', {
        connection_id: 'c-github-pw',
        tool: 'GITHUB_LIST_REPOS',
        arguments: {},
      })
    ).rejects.toThrow(/failed/i);
    await assertSessionAlive(page);

    await setMockBehavior({ composioDeleteFails: '500' });
    await expect(
      callCoreRpc('openhuman.composio_delete_connection', { connection_id: 'c-discord-pw' })
    ).rejects.toThrow(/failed/i);
    await assertSessionAlive(page);
  });
});
