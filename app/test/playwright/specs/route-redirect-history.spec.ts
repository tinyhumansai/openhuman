import { expect, type Page, test } from '@playwright/test';

import { bootAuthenticatedPage, waitForAppReady } from '../helpers/core-rpc';

// `bootAuthenticatedPage` runs in every `beforeEach` and costs 30-60s against a
// locally-built debug core — the sidebar suite's first test measured 59.1s
// against the config's 60s non-CI budget, and this suite's first two tests blew
// it outright ("Test timeout of 60000ms exceeded while running beforeEach").
// The work is the harness's, not the assertions': raise the ceiling here rather
// than in the shared playwright.config.ts, which is not this worker's to edit.
test.describe.configure({ timeout: 180_000 });

/**
 * Retired routes redirect with `replace`, and the browser Back button must not
 * hand the user back to the retired path.
 *
 * This is the half jsdom cannot prove. `AppRoutes.connections-flows.test.tsx`
 * and `navigation.spec.ts` both assert the landing path after a redirect, and
 * both would pass identically if every `<Navigate>` lost its `replace` prop —
 * because the thing `replace` changes is the session history stack, not the
 * destination. The user-visible contract is: go to a retired URL, land on the
 * live one, press Back, and you go to wherever you were BEFORE — not into a
 * redirect that fires again and traps you.
 *
 * Without `replace` the retired entry stays on the stack, so Back re-enters it,
 * the redirect fires again, and the user is pinned. That bug is invisible to
 * every existing test in this repo.
 */

const hash = (page: Page) => page.evaluate(() => window.location.hash);

/** Retired path → the live path it must land on. */
const REDIRECTS: ReadonlyArray<readonly [string, string]> = [
  ['/skills', '/connections'],
  ['/channels', '/connections'],
  ['/routines', '/flows'],
  // Two hops, not one: `/webhooks` redirects to `/settings/integrations`
  // (`AppRoutes.tsx:238`), which is ITSELF a redirect to `/connections`
  // (`settingsRouteElements.tsx:129`). The final landing is what the user sees,
  // so that is what this asserts. Measured in a real browser — the declared
  // target alone would have been wrong, and the jsdom suite could not tell,
  // because it never mounts the nested settings routes.
  ['/webhooks', '/connections'],
  ['/home', '/chat'],
  ['/activity', '/settings/notifications'],
  ['/intelligence', '/settings/notifications'],
  // The retired unified-chat aliases. `/accounts` predates the /chat merge;
  // `/feedback` moved into Settings.
  ['/accounts', '/chat'],
  ['/feedback', '/settings/feedback'],
];

// Guard against this list silently falling behind the route table: every
// top-level `<Navigate>` in AppRoutes.tsx should appear above. There are nine
// (AppRoutes.tsx lines 75, 141, 142, 170, 184, 188, 202, 215, 238) and the
// first version of this spec covered seven — the two chat/settings aliases
// were simply missed. A count assertion is a cheap tripwire for the next one.
const EXPECTED_TOP_LEVEL_REDIRECTS = 9;

test.describe('Retired routes redirect without trapping the Back button', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-redirect-history-user', '/brain');
  });

  for (const [retired, live] of REDIRECTS) {
    test(`${retired} → ${live}, and Back leaves rather than re-entering`, async ({ page }) => {
      // Establish a known previous page so "Back" has somewhere real to go.
      await page.goto('/#/brain');
      await waitForAppReady(page);
      await expect.poll(() => hash(page)).toMatch(/^#\/brain/);

      await page.goto(`/#${retired}`);
      await waitForAppReady(page);
      await expect.poll(() => hash(page)).toMatch(new RegExp(`^#${live.replace(/\//g, '\\/')}`));

      await page.goBack();

      // The retired path must NOT be where we land. If `replace` were dropped,
      // Back re-enters `retired`, the redirect fires again, and the hash
      // settles on `live` a second time — the user cannot get out.
      await expect
        .poll(() => hash(page), { timeout: 10_000 })
        .not.toMatch(new RegExp(`^#${retired.replace(/\//g, '\\/')}`));

      // And we should be back where we came from.
      await expect.poll(() => hash(page), { timeout: 10_000 }).toMatch(/^#\/brain/);
    });
  }

  test('this spec covers every top-level redirect the route table declares', () => {
    expect(REDIRECTS).toHaveLength(EXPECTED_TOP_LEVEL_REDIRECTS);
  });
});

test.describe('The /channels redirect carries its tab selector through a real navigation', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-redirect-tab-user', '/brain');
  });

  test('/channels lands on the Connections messaging tab, not the default tab', async ({
    page,
  }) => {
    // `/channels` was an orphaned standalone page; the Messaging tab of
    // Connections replaced it. The redirect hardcodes `?tab=messaging` in its
    // target, so losing the query drops the user on the Welcome tab instead —
    // a silent regression, since the page still renders perfectly.
    await page.goto('/#/channels');
    await waitForAppReady(page);

    await expect.poll(() => hash(page)).toMatch(/^#\/connections/);
    await expect.poll(() => hash(page)).toContain('tab=messaging');
  });

  test('/skills?tab=… currently DROPS the query — pinned, see W5 BUG-1', async ({ page }) => {
    // `AppRoutes.tsx` claims twice that this redirect "preserves ?tab= deep
    // links". It does not: `<Navigate to="/connections" replace />` takes a
    // bare path string with no search component. The knock-on is that the
    // legacy alias table in `pages/Skills.tsx` — written, per its own comment,
    // so `/skills?tab=composio` keeps working after the redirect — is
    // unreachable from this route.
    //
    // Pinned as CURRENT behaviour so it cannot deepen unnoticed. When it is
    // fixed, flip this to expect `tab=messaging` and delete the note.
    await page.goto('/#/skills?tab=messaging');
    await waitForAppReady(page);

    await expect.poll(() => hash(page)).toMatch(/^#\/connections/);
    await expect.poll(() => hash(page)).not.toContain('tab=messaging');
  });
});
