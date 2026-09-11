import { expect, type Page, test } from '@playwright/test';

import { bootAuthenticatedPage, dismissWalkthroughIfPresent } from '../helpers/core-rpc';

// `bootAuthenticatedPage` runs in every `beforeEach` and costs 30-60s against a
// locally-built debug core — the sidebar suite's first test measured 59.1s
// against the config's 60s non-CI budget, and this suite's first two tests blew
// it outright ("Test timeout of 60000ms exceeded while running beforeEach").
// The work is the harness's, not the assertions': raise the ceiling here rather
// than in the shared playwright.config.ts, which is not this worker's to edit.
test.describe.configure({ timeout: 180_000 });

/**
 * Root-shell sidebar: routing by click, the active-row marker, and the
 * collapse / icon-only rail (openhuman#5676).
 *
 * Why this is not covered by `navigation.spec.ts`: that spec drives every
 * route with `page.goto('/#/route')` and asserts the hash plus a >50-character
 * `#root`. It never touches the sidebar. Nothing in the repo clicks a nav row,
 * asserts which row is marked current, or collapses the shell — so the entire
 * `matchActive` table in `SidebarNav.tsx:33-38` and the whole collapsed-rail
 * path are unexercised.
 *
 * Markers used, all from the product rather than added for the test:
 *   - each row is a `SidebarMenuButton`, which sets `data-active="true|false"`
 *     and `aria-current="page"` (`components/ui/Sidebar.tsx:596-597`)
 *   - rows carry `data-walkthrough="tab-<id>"` from `NAV_TABS`
 *   - the sidebar column is `data-testid="root-shell-sidebar"` and the
 *     primitive stamps `data-state="expanded|collapsed"`
 *   - collapse is the "Hide sidebar" button; reopen is
 *     `data-testid="root-shell-reopen"`
 */

const row = (page: Page, id: string) => page.locator(`[data-walkthrough="tab-${id}"]`);
const sidebar = (page: Page) => page.locator('[data-testid="root-shell-sidebar"]');

/** The nav row currently marked as the active route, by its tab id. */
async function activeRowId(page: Page): Promise<string | null> {
  return page.evaluate(() => {
    const el = document.querySelector('[data-walkthrough^="tab-"][data-active="true"]');
    return el?.getAttribute('data-walkthrough')?.replace('tab-', '') ?? null;
  });
}

const hash = (page: Page) => page.evaluate(() => window.location.hash);

test.describe('App shell — sidebar navigation', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-app-shell-sidebar-user');
    await dismissWalkthroughIfPresent(page);
  });

  test('clicking each nav row routes there and marks exactly that row current', async ({
    page,
  }) => {
    // `rewards` is `cloudOnly` in NAV_TABS, so it is deliberately absent for a
    // session without cloud — asserted separately below rather than assumed.
    for (const [id, expectedHash] of [
      ['brain', '/brain'],
      ['flows', '/flows'],
      ['connections', '/connections'],
      ['chat', '/chat'],
    ] as const) {
      await row(page, id).click();

      await expect.poll(() => hash(page)).toMatch(new RegExp(`^#${expectedHash}`));

      // Exactly one row is current, and it is this one. The "exactly one" half
      // matters: `matchActive` uses prefix rules for /chat, /settings and
      // /flows, so a sloppy rule lights two rows at once and the user loses
      // any sense of where they are.
      await expect.poll(() => activeRowId(page)).toBe(id);
      await expect(page.locator('[data-walkthrough^="tab-"][data-active="true"]')).toHaveCount(1);
      await expect(row(page, id)).toHaveAttribute('aria-current', 'page');
    }
  });

  test('a deep sub-route keeps its parent nav row highlighted', async ({ page }) => {
    // `matchActive` gives /flows a prefix rule specifically so the canvas at
    // /flows/:id keeps the Flows row lit. Nothing tested that.
    await page.goto('/#/flows/some-flow-id');
    await expect.poll(() => activeRowId(page)).toBe('flows');

    await page.goto('/#/chat/some-thread-id');
    await expect.poll(() => activeRowId(page)).toBe('chat');
  });

  test('the Rewards row is present for a cloud session and routes', async ({ page }) => {
    // Asserted, not recorded. The first version accepted `count === 0` as a
    // pass, which meant a regressed gate, a gate that never becomes ready, or a
    // deleted row all counted as success — precisely the failures the test
    // names (#5887, Codex).
    //
    // This fixture IS a cloud session, so the gate must open. `useCloudNavGate`
    // requires `isReady && sessionToken && !isLocalSessionToken(token)`
    // (`useCloudNavGate.ts:26-28`), and `isLocalSessionToken` is true only for a
    // token whose third dot-part is literally `local`
    // (`utils/localSession.ts:32-36`). `bootAuthenticatedPage` installs
    // `buildBypassJwt`, which ends `.sig` (`helpers/core-rpc.ts:17-22`) — so the
    // token is non-local and Rewards must be offered.
    await expect(row(page, 'rewards')).toHaveCount(1);

    await row(page, 'rewards').click();
    await expect.poll(() => hash(page)).toMatch(/^#\/rewards/);
    await expect.poll(() => activeRowId(page)).toBe('rewards');
  });
});

test.describe('App shell — collapse and the icon-only rail (#5676)', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-app-shell-collapse-user');
    await dismissWalkthroughIfPresent(page);
  });

  test('collapsing hides the labels, keeps the rail, and reopening restores', async ({ page }) => {
    const shell = sidebar(page);
    await expect(shell).toHaveAttribute('data-state', 'expanded');

    // Labels are readable while expanded.
    const chatLabel = row(page, 'chat');
    await expect(chatLabel).toBeVisible();
    const expandedWidth = await shell.evaluate(el => el.getBoundingClientRect().width);
    expect(expandedWidth).toBeGreaterThan(120);

    // The row labels are readable while expanded — `SidebarMenuLabel` renders
    // them as `[data-slot="sidebar-menu-label"]` spans.
    const labels = page.locator('[data-slot="sidebar-menu-label"]');
    expect(await labels.count()).toBeGreaterThan(0);

    await page.getByRole('button', { name: 'Hide sidebar' }).click();

    await expect(shell).toHaveAttribute('data-state', 'collapsed');
    // The test is called "collapsing hides the labels" and did not check any
    // label — a regression leaving them visible passed (#5887, CodeRabbit).
    // Collapsed swaps `SidebarNav` for `CollapsedNavRail`, which renders icons
    // with `aria-label` and no label spans, so the count goes to zero.
    await expect(labels).toHaveCount(0);
    const collapsedWidth = await shell.evaluate(el => el.getBoundingClientRect().width);
    // The rail is still there — collapsed is icon-only, not gone. A regression
    // that unmounts the column instead of narrowing it passes any assertion
    // written only against `data-state`.
    expect(collapsedWidth).toBeGreaterThan(0);
    expect(collapsedWidth).toBeLessThan(expandedWidth);

    // The reopen affordance is the thing that makes collapse reversible; if it
    // is missing the user is stranded in the rail.
    const reopen = page.getByTestId('root-shell-reopen');
    await expect(reopen).toBeVisible();

    await reopen.click();
    await expect(shell).toHaveAttribute('data-state', 'expanded');
    await expect
      .poll(async () => shell.evaluate(el => el.getBoundingClientRect().width))
      .toBeGreaterThan(120);
  });

  test('navigation still works from the collapsed rail', async ({ page }) => {
    // The point of an icon-only rail is that it is still a rail. If collapsing
    // strands the user, the feature is worse than no collapse at all.
    await page.getByRole('button', { name: 'Hide sidebar' }).click();
    await expect(sidebar(page)).toHaveAttribute('data-state', 'collapsed');

    const railRow = row(page, 'connections');
    await expect(railRow).toBeVisible();
    await railRow.click();

    await expect.poll(() => hash(page)).toMatch(/^#\/connections/);
    await expect.poll(() => activeRowId(page)).toBe('connections');
    // Still collapsed after navigating — a rail that springs back open on
    // every click is not a collapsed rail.
    await expect(sidebar(page)).toHaveAttribute('data-state', 'collapsed');
  });
});
