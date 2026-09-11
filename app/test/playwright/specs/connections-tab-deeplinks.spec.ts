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
 * The `/skills` → `/connections` redirect did once drop the query — `<Navigate
 * to="/connections" replace />` is a fixed string with no search, and React
 * Router does not carry the current query across it. #5924 replaced it with
 * `ForwardSearch`, which copies `search` and `hash` onto the destination; the
 * `?tab=` tests below assert that forwarding rather than the old defect.
 *
 * `/webhooks` forwards the same way but in TWO hops — via
 * `/settings/integrations`, which is itself a redirect (#5939). Both hops used
 * a bare `<Navigate>`, so a fix to only the first would have handed the query
 * to the second and had it dropped there; the `/webhooks` tests below assert
 * the FINAL destination for that reason.
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

  test('/skills?tab=channels carries the tab through the redirect', async ({ page }) => {
    // Fixed by #5924. This test previously pinned the BUG — it asserted the
    // query was dropped and the overview rendered — and was left un-flipped
    // when the fix landed, so it asserted the opposite of shipped behaviour.
    // `AppRoutes.tsx` now redirects through `ForwardSearch`, which copies the
    // current `search` and `hash` onto the destination.
    //
    // Assert the SELECTED tab, not merely that a Channels nav row is visible:
    // that row renders on every tab and would prove nothing.
    await openRoute(page, 'pw-skills-tab-forward', '/skills?tab=channels', '/connections');
    await expect.poll(() => currentHash(page), { timeout: 15_000 }).toContain('/connections');

    await expect.poll(() => currentHash(page), { timeout: 15_000 }).toContain('tab=channels');
    await expectSelectedTab(page, 'channels');
  });

  test('/skills?tab=mcp forwards any tab, not just one hard-coded value', async ({ page }) => {
    // A second tab so the forward cannot be satisfied by a literal. `mcp` is a
    // canonical id (Skills.tsx), reached here only via the redirect.
    await openRoute(page, 'pw-skills-tab-forward-mcp', '/skills?tab=mcp', '/connections');
    await expect.poll(() => currentHash(page), { timeout: 15_000 }).toContain('tab=mcp');
    await expectSelectedTab(page, 'mcp');
  });

  test('/skills?tab=mcp#fragment forwards the fragment as well as the query', async ({ page }) => {
    // `ForwardSearch` appends `hash` as well as `search`, and nothing here
    // exercised that half. `AppRoutes.skills.test.tsx` does assert
    // `loc.hash === '#section-mcp'`, but under `MemoryRouter`, which never
    // parses `window.location.hash` at all — so it cannot show that a real
    // HashRouter round-trips a fragment nested inside the routing hash
    // (`#/connections?tab=mcp#section-mcp`). That is the part only a browser
    // can answer, and it is the mechanism `/webhooks` relies on for
    // `#delivery-3`-style deep links.
    await openRoute(page, 'pw-skills-hash-forward', '/skills?tab=mcp#section-mcp', '/connections');
    await expect.poll(() => currentHash(page), { timeout: 15_000 }).toContain('tab=mcp');
    expect(await currentHash(page)).toContain('#section-mcp');
    await expectSelectedTab(page, 'mcp');
  });

  test('/skills with no query still lands on the overview', async ({ page }) => {
    // The forward must not invent a query where the user supplied none.
    await openRoute(page, 'pw-skills-no-query', '/skills', '/connections');
    const hash = await currentHash(page);
    expect(hash).not.toContain('tab=');
    await expectSelectedTab(page, 'welcome');
  });

  test('/webhooks?tab= survives BOTH hops of its redirect', async ({ page }) => {
    // #5939 (closes #5908). `/webhooks` is a TWO-hop redirect:
    //
    //   /webhooks              -> /settings/integrations   (AppRoutes.tsx:239)
    //   /settings/integrations -> /connections             (settingsRouteElements.tsx:130)
    //
    // Both hops used a bare `<Navigate>`, so fixing only the first would have
    // handed the query to hop two and had it dropped there instead. That is why
    // this asserts the FINAL destination rather than the intermediate one — a
    // half-fix passes an assertion on `/settings/integrations` and still loses
    // the deep link the user actually followed.
    await openRoute(page, 'pw-webhooks-tab-keep', '/webhooks?tab=channels', '/connections');
    await expect.poll(() => currentHash(page), { timeout: 15_000 }).toContain('/connections');

    expect(await currentHash(page)).toContain('tab=channels');
    await expectSelectedTab(page, 'channels');
  });

  test('/webhooks carries the fragment through as well as the query', async ({ page }) => {
    // `ForwardSearch` copies `hash` alongside `search`. Under HashRouter the
    // fragment is the part after a SECOND `#`, so this also pins that the two
    // are not conflated — a redirect that forwarded only `search` would land on
    // the right tab and still drop the anchor.
    await openRoute(
      page,
      'pw-webhooks-fragment-keep',
      '/webhooks?tab=channels#delivery-3',
      '/connections'
    );
    await expect.poll(() => currentHash(page), { timeout: 15_000 }).toContain('/connections');

    const hash = await currentHash(page);
    expect(hash).toContain('tab=channels');
    expect(hash).toContain('#delivery-3');
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
