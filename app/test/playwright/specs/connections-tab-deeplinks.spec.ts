import { expect, test } from '@playwright/test';

import {
  bootRuntimeReadyExistingSessionPage,
  bootRuntimeReadyGuestPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  signInViaBypassUser,
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
  const snapshot = await callCoreRpc<{
    result?: { currentUser?: { _id?: string | null } | null };
    currentUser?: { _id?: string | null } | null;
  }>('openhuman.app_state_snapshot', {});
  const currentUser = (snapshot.result ?? snapshot).currentUser;

  if (currentUser?._id) {
    await bootRuntimeReadyExistingSessionPage(page);
  } else {
    await bootRuntimeReadyGuestPage(page);
    await signInViaBypassUser(page, userId);
  }
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

test('Connections deep links preserve their selected pane, search, and fragment', async ({
  page,
}) => {
  await openRoute(page, 'pw-connection-deeplinks', '/connections');

  const navigate = async (route: string, settlesOn = '/connections') => {
    await page.evaluate(target => {
      window.location.hash = target;
    }, route);
    await expect.poll(() => currentHash(page), { timeout: 15_000 }).toContain(settlesOn);
  };
  const expectWelcome = async () => {
    await expect(page.getByTestId('connections-welcome')).toBeVisible();
    await expect(page.locator('[data-testid^="two-pane-nav-"][aria-current="page"]')).toHaveCount(
      0
    );
  };

  await page.getByTestId('two-pane-nav-channels').click();
  await expect.poll(() => currentHash(page), { timeout: 10_000 }).toContain('tab=channels');
  await page.getByTestId('two-pane-nav-mcp').click();
  await expect.poll(() => currentHash(page), { timeout: 10_000 }).toContain('tab=mcp');
  await expect(
    page
      .getByRole('searchbox')
      .or(page.getByPlaceholder(/search/i))
      .first()
  ).toBeVisible();

  await navigate('/connections?tab=channels');
  await expectSelectedTab(page, 'channels');
  await page.reload();
  await expectSelectedTab(page, 'channels');

  await navigate('/connections?tab=messaging');
  await expectSelectedTab(page, 'channels');
  await navigate('/connections?tab=not-a-real-tab');
  await expectWelcome();

  await navigate('/skills');
  expect(await currentHash(page)).not.toContain('tab=');
  await expectWelcome();
  await navigate('/skills?tab=channels');
  await expectSelectedTab(page, 'channels');
  await navigate('/skills?tab=mcp');
  await expectSelectedTab(page, 'mcp');
  await navigate('/skills?tab=mcp#section-mcp');
  expect(await currentHash(page)).toContain('#section-mcp');
  await expectSelectedTab(page, 'mcp');

  await navigate('/webhooks?tab=channels#delivery-3');
  expect(await currentHash(page)).toContain('#delivery-3');
  await expectSelectedTab(page, 'channels');
  await navigate('/channels');
  await expect.poll(() => currentHash(page), { timeout: 15_000 }).toContain('tab=messaging');
  await expectSelectedTab(page, 'channels');
});
