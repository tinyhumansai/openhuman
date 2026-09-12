import { expect, test } from '@playwright/test';

import {
  bootRuntimeReadyGuestPage,
  dismissWalkthroughIfPresent,
  signInViaCallbackToken,
  waitForAppReady,
} from '../helpers/core-rpc';

/**
 * The Connections tab **alias table**, and the redirect that silently depends
 * on it.
 *
 * `connections-tab-deeplinks.spec.ts` (mine) asserts what the URL says. This
 * file asserts something different and untested: which tab a given `?tab=`
 * value actually RESOLVES to. `Skills.tsx:517-543` accepts twelve canonical
 * values, then maps four legacy ones — `apps → composio`, `messaging →
 * channels`, `tools → mcp`, `explorer → skills` — and falls back to `welcome`
 * for anything it does not recognise.
 *
 * The reason this is worth pinning is `AppRoutes.tsx:188`:
 *
 *     <Route path="/channels" element={<Navigate to="/connections?tab=messaging" replace />} />
 *
 * The redirect targets a **legacy alias**, not a canonical value. So `/channels`
 * — a route the app ships and links to — works only for as long as the
 * back-compat table keeps an entry that looks, from inside `Skills.tsx`, purely
 * historical. Delete `messaging` while tidying up "old" aliases and `/channels`
 * does not error: it silently lands on the Welcome overview, because the
 * fallback swallows an unrecognised value by design. The tab-deeplinks spec
 * would not catch it either — the hash still reads `?tab=messaging`, which is
 * all that one asserts. `alias resolution` and `URL correctness` are genuinely
 * two contracts, and only the first is load-bearing for `/channels`.
 *
 * Assertions use the nav row's `aria-current`, not panel content
 * (`TwoPaneNav.tsx:97-98` — `data-testid="two-pane-nav-<value>"` and
 * `aria-current="page"` on the active row). That is deliberate: several panels
 * fetch on mount, and asserting rendered content would make this file's result
 * depend on an upstream registry rather than on the resolver. Selection is
 * decided by a `useMemo` over `location.search` and nothing else.
 *
 * NOTE ON SCOPE: `apps → composio` is the one alias NOT covered. Selecting the
 * Composio tab downloads the `tinyconnectors` module from a GitHub release, and
 * a failed download is terminal for the core process (W3-ui-bugs.md §3) — it
 * would take the rest of the file down with it. It is listed here so its
 * absence reads as a decision rather than an oversight.
 */

/** The nav row for a canonical tab id. */
const navRow = (page: import('@playwright/test').Page, tab: string) =>
  page.getByTestId(`two-pane-nav-${tab}`);

/** Assert `tab` is the selected row, and that exactly one row is selected. */
async function expectSelectedTab(page: import('@playwright/test').Page, tab: string) {
  await expect(navRow(page, tab)).toHaveAttribute('aria-current', 'page', { timeout: 15_000 });
  // One selected row, not "the one I asked about happens to be marked". A
  // resolver returning several actives, or marking every row, would otherwise
  // satisfy the assertion above.
  await expect(page.locator('[data-testid^="two-pane-nav-"][aria-current="page"]')).toHaveCount(1);
}

async function openConnections(
  page: import('@playwright/test').Page,
  userId: string,
  route: string
) {
  await bootRuntimeReadyGuestPage(page);
  await signInViaCallbackToken(page, userId);
  await page.evaluate(target => {
    try {
      localStorage.setItem('openhuman:walkthrough_completed', 'true');
      localStorage.removeItem('openhuman:walkthrough_pending');
    } catch {}
    window.location.hash = target;
  }, route);
  await expect
    .poll(async () => page.evaluate(() => window.location.hash), { timeout: 15_000 })
    .toContain('/connections');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
}

test.describe('Connections — legacy ?tab= aliases resolve to their canonical tab', () => {
  test('?tab=messaging selects Channels', async ({ page }) => {
    await openConnections(page, 'pw-alias-messaging', '/connections?tab=messaging');
    await expectSelectedTab(page, 'channels');
  });

  test('?tab=tools selects MCP', async ({ page }) => {
    await openConnections(page, 'pw-alias-tools', '/connections?tab=tools');
    await expectSelectedTab(page, 'mcp');
  });

  test('?tab=explorer selects Skills', async ({ page }) => {
    await openConnections(page, 'pw-alias-explorer', '/connections?tab=explorer');
    await expectSelectedTab(page, 'skills');
  });

  test('a canonical value still wins directly', async ({ page }) => {
    // The control. If this failed alongside the alias tests, the fault would be
    // in tab selection generally rather than in the alias table.
    await openConnections(page, 'pw-alias-canonical', '/connections?tab=llm');
    await expectSelectedTab(page, 'llm');
  });
});

test.describe('Connections — unrecognised ?tab= falls back rather than breaking', () => {
  test('an unknown tab value lands on Welcome, not a blank pane', async ({ page }) => {
    // A stale bookmark naming a tab that no longer exists must degrade to the
    // overview. `Skills.tsx:542` returns 'welcome' for anything unmatched.
    await openConnections(page, 'pw-alias-unknown', '/connections?tab=zzz-not-a-tab');
    await expectSelectedTab(page, 'welcome');
  });

  test('no ?tab= at all lands on Welcome', async ({ page }) => {
    await openConnections(page, 'pw-alias-none', '/connections');
    await expectSelectedTab(page, 'welcome');
  });
});

test.describe('Connections — /channels depends on the alias table', () => {
  test('/channels lands on the Channels TAB, not merely a messaging URL', async ({ page }) => {
    // The two-layer contract in one assertion: `AppRoutes.tsx:188` rewrites
    // /channels to `?tab=messaging`, and only the alias table turns that into
    // the Channels tab. Asserting the hash alone (as the deeplinks spec does)
    // passes even when the second layer is gone.
    await openConnections(page, 'pw-alias-channels-route', '/channels');
    await expect
      .poll(async () => page.evaluate(() => window.location.hash), { timeout: 15_000 })
      .toContain('tab=messaging');
    await expectSelectedTab(page, 'channels');
  });
});
