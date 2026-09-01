import { expect, test } from '@playwright/test';

import {
  bootRuntimeReadyGuestPage,
  dismissWalkthroughIfPresent,
  signInViaCallbackToken,
  waitForAppReady,
} from '../helpers/core-rpc';

/**
 * Deep links into the Connections page, driven in a real browser.
 *
 * `skills-registry.spec.ts` already clicks each tab and asserts the panel that
 * renders. What no spec checks is the URL: whether the address bar reflects the
 * tab after a click, and — the half that actually reaches users — whether a
 * bookmarked deep link lands on the tab it names.
 *
 * The `/skills` → `/connections` redirect (`AppRoutes.tsx:169-170`) carries a
 * comment saying it "preserves ?tab= deep links". It does not: `<Navigate
 * to="/connections" replace />` is a fixed string with no search, and React
 * Router does not carry the current query across it. See the `?tab=` tests
 * below, which pin the CURRENT behaviour and are annotated with what it should
 * be, rather than asserting the fix ahead of it.
 *
 * NOTE ON SCOPE: nothing here opens the Composio tab. Doing so downloads the
 * `tinyconnectors` module from a GitHub release, and a failed download is
 * terminal for the core process — which takes the rest of the file with it.
 * That is an environment constraint of this lane, recorded in W3-ui-bugs.md §3.
 *
 * Tab ids come from `pages/Skills.tsx:517-543`: canonical `welcome | composio |
 * channels | mcp | skills | llm | voice | embeddings | search | usage |
 * composio-key | wallet`, plus the legacy aliases `apps → composio`,
 * `messaging → channels`, `tools → mcp`, `explorer → skills`.
 */

const HASH = (route: string) => `#${route}`;

/**
 * Boot a signed-in page parked on `route` (a hash route, e.g. `/connections`).
 *
 * `settlesOn` is the path the hash is expected to END on, which differs from
 * `route` whenever the route is a redirect: `/skills` never appears in the hash
 * because `<Navigate>` replaces it before the first poll can observe it.
 */
async function openRoute(
  page: import('@playwright/test').Page,
  userId: string,
  route: string,
  settlesOn?: string
) {
  await bootRuntimeReadyGuestPage(page);
  await signInViaCallbackToken(page, userId);
  await page.evaluate(
    ({ target }) => {
      try {
        localStorage.setItem('openhuman:walkthrough_completed', 'true');
        localStorage.removeItem('openhuman:walkthrough_pending');
      } catch {}
      window.location.hash = target;
    },
    { target: route }
  );
  await expect
    .poll(async () => page.evaluate(() => window.location.hash), { timeout: 15_000 })
    .toContain(settlesOn ?? route.split('?')[0]);
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
}

const currentHash = (page: import('@playwright/test').Page) =>
  page.evaluate(() => window.location.hash);

/**
 * Assert which nav row is selected.
 *
 * `TwoPaneNav.tsx:98` marks the active row with `aria-current="page"`. Asserting
 * a row is *visible* says nothing — every row is visible on every tab — so any
 * check of "did we land on the right tab" has to read the selection.
 */
async function expectSelectedTab(page: import('@playwright/test').Page, tab: string) {
  await expect(page.getByTestId(`two-pane-nav-${tab}`)).toHaveAttribute('aria-current', 'page', {
    timeout: 15_000,
  });
  await expect(page.locator('[data-testid^="two-pane-nav-"][aria-current="page"]')).toHaveCount(1);
}

test.describe('Connections — the URL follows the tab', () => {
  test('clicking a tab writes ?tab= into the address bar', async ({ page }) => {
    await openRoute(page, 'pw-conn-url-click', '/connections');

    // Landing default is the Welcome overview (Skills.tsx:542).
    await expect(page.getByTestId('two-pane-nav-composio')).toBeVisible();

    await page.getByTestId('two-pane-nav-channels').click();
    await expect.poll(() => currentHash(page), { timeout: 10_000 }).toContain('tab=channels');

    await page.getByTestId('two-pane-nav-mcp').click();
    await expect.poll(() => currentHash(page), { timeout: 10_000 }).toContain('tab=mcp');
  });

  test('the tab in the URL is the tab that renders', async ({ page }) => {
    await openRoute(page, 'pw-conn-url-render', '/connections');

    await page.getByTestId('two-pane-nav-mcp').click();
    await expect.poll(() => currentHash(page), { timeout: 10_000 }).toContain('tab=mcp');
    // MCP panel: a search field plus the All/Installed/Registry filter row.
    await expect(
      page
        .getByRole('searchbox')
        .or(page.getByPlaceholder(/search/i))
        .first()
    ).toBeVisible();

    await page.getByTestId('two-pane-nav-channels').click();
    await expect.poll(() => currentHash(page), { timeout: 10_000 }).toContain('tab=channels');
    await expect(page.getByText(/Telegram|Discord|Slack/).first()).toBeVisible();
  });

  test('a reload on a ?tab= URL comes back to the same tab', async ({ page }) => {
    // The whole point of putting the tab in the URL: it has to survive a
    // refresh, not just a click.
    await openRoute(page, 'pw-conn-url-reload', '/connections?tab=channels');
    await expect(page.getByText(/Telegram|Discord|Slack/).first()).toBeVisible();

    await page.reload();
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);

    await expect.poll(() => currentHash(page), { timeout: 15_000 }).toContain('tab=channels');
    await expect(page.getByText(/Telegram|Discord|Slack/).first()).toBeVisible();
  });
});

test.describe('Connections — deep links land on the named tab', () => {
  // Only `channels` here. `composio` is deliberately excluded: opening that tab
  // calls `composio.list_toolkits`, which loads the `tinyconnectors` native
  // module by downloading it from a GitHub release. In a sandbox with no
  // network that fails, and the core treats it as "terminal for the running
  // process" — the process goes down and every later test in the file gets
  // ECONNREFUSED on 127.0.0.1:17788. See W3-ui-bugs.md §3.
  for (const [param, marker] of [['channels', /Telegram|Discord|Slack/]] as const) {
    test(`/connections?tab=${param} opens that tab directly`, async ({ page }) => {
      await openRoute(page, `pw-conn-deep-${param}`, `/connections?tab=${param}`);
      await expect(page.getByText(marker).first()).toBeVisible();
      expect(await currentHash(page)).toContain(`tab=${param}`);
    });
  }

  test('a legacy alias resolves to its canonical tab', async ({ page }) => {
    // `messaging` is the pre-rename name for `channels` (Skills.tsx:537-540).
    // A bookmark from the old UI must still land on the messaging connectors.
    await openRoute(page, 'pw-conn-legacy-alias', '/connections?tab=messaging');
    await expect(page.getByText(/Telegram|Discord|Slack/).first()).toBeVisible();
  });

  test('an unknown tab value falls back to the overview rather than a blank pane', async ({
    page,
  }) => {
    await openRoute(page, 'pw-conn-unknown-tab', '/connections?tab=not-a-real-tab');
    // Assert WHICH tab the fallback selected, not merely that the nav rendered.
    // The nav rows are present on every tab, so the earlier form passed if an
    // unknown value selected MCP, Channels, or anything else non-blank — it
    // could not distinguish a working fallback from a wrong one. Caught in
    // review by `coderabbitai`.
    await expectSelectedTab(page, 'welcome');
  });
});

test.describe('Connections — the /skills back-compat redirect', () => {
  test('/skills lands on /connections', async ({ page }) => {
    await openRoute(page, 'pw-skills-redirect', '/skills', '/connections');
    await expect.poll(() => currentHash(page), { timeout: 15_000 }).toContain('/connections');
    await expect(page.getByTestId('two-pane-nav-composio')).toBeVisible();
  });

  test('BUG: /skills?tab=channels drops the tab and lands on the overview', async ({ page }) => {
    // This pins CURRENT behaviour, which is wrong. `AppRoutes.tsx:169` says the
    // redirect "preserves ?tab= deep links"; `<Navigate to="/connections" />`
    // carries no search, so the query is dropped and `activeTab` falls through
    // to its `welcome` default.
    //
    // Expected once fixed: the hash contains `tab=channels` and the messaging
    // connectors are visible. Flip the two assertions below when that lands.
    await openRoute(page, 'pw-skills-tab-drop', '/skills?tab=channels', '/connections');
    await expect.poll(() => currentHash(page), { timeout: 15_000 }).toContain('/connections');

    const hash = await currentHash(page);
    expect(hash).not.toContain('tab=channels');
    // The user asked for messaging and got the overview instead. Assert the
    // SELECTED tab: the previous form asserted that the Channels nav row was
    // visible, which is true on every tab and therefore proved nothing about
    // the harm this test exists to document.
    await expectSelectedTab(page, 'welcome');
  });

  test('/channels keeps working, because its redirect names the tab explicitly', async ({
    page,
  }) => {
    // The contrast that proves the /skills case is a bug and not a limitation:
    // `AppRoutes.tsx:188` redirects to `/connections?tab=messaging` — a literal
    // search string — and that one does arrive on the right tab.
    await openRoute(page, 'pw-channels-redirect', '/channels', '/connections');
    await expect.poll(() => currentHash(page), { timeout: 15_000 }).toContain('/connections');
    expect(await currentHash(page)).toContain('tab=messaging');
    await expect(page.getByText(/Telegram|Discord|Slack/).first()).toBeVisible();
  });
});
