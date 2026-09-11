import { expect, type Page, test } from '@playwright/test';

import { bootAuthenticatedPage, dismissWalkthroughIfPresent } from '../helpers/core-rpc';

// See app-shell-sidebar.spec.ts for why the budget is raised here rather than
// in the shared playwright.config.ts.
test.describe.configure({ timeout: 180_000 });

/**
 * Sidebar drag-resize, arrow-key resize, and the seam indicator.
 *
 * These are acceptance criteria 3 and 4 of openhuman#5676. **This spec does NOT
 * close that issue** — read the note at the bottom of this comment before
 * claiming it does.
 *
 * #5676 asks for a *visual* pass on a real desktop build, and its four criteria
 * split cleanly by what a browser can reach:
 *
 *   AC-1 no native webview punch-through in the collapsed state — OUT OF REACH.
 *        The web lane is a browser tab; there is no native webview to punch
 *        through, so a green run here says nothing about it.
 *   AC-2 macOS traffic lights stay clear of the collapsed rail — OUT OF REACH.
 *        No window chrome exists in this lane.
 *   AC-3 the `SidebarRail` seam paints `bg-line-chrome` rather than `bg-line`
 *        on hover/focus — REACHABLE. The issue notes that unit tests "only
 *        assert the class name is applied, never its rendered colour";
 *        `getComputedStyle` in a real engine is exactly the missing instrument.
 *   AC-4 drag-resize works, the width persists across a restart, and arrow-key
 *        steps still apply — REACHABLE, and untested anywhere today.
 *
 * So this covers the two behavioural criteria and leaves the two compositing
 * ones for the desktop pass the issue actually asks for.
 */

const sidebar = (page: Page) => page.locator('[data-testid="root-shell-sidebar"]');
const rail = (page: Page) => page.locator('[data-testid="root-shell-divider"]');

const widthOf = (page: Page) =>
  sidebar(page).evaluate(el => Math.round(el.getBoundingClientRect().width));

/**
 * The rail itself has **zero layout width** — `w-0`, deliberately, so it adds
 * nothing to the content card's left gutter (`components/ui/Sidebar.tsx:330`).
 * That makes Playwright treat it as *not visible*: `toBeVisible()` fails and
 * `hover()` can never act on it, however correct the element is.
 *
 * Two absolutely-positioned children carry the real geometry:
 *   nth(0) — the widened hit area (`-left-1 -right-1`), what a user points at
 *   nth(1) — the 1px seam, what carries the colour classes
 *
 * So: point at the hit area, read colour off the seam, and assert the rail's
 * presence by count rather than by visibility.
 *
 * The LEFT-half-only caveat this comment used to carry is FIXED (#5906). It
 * read: measured with `elementFromPoint` at a 224px sidebar edge, x=221 and
 * x=222 reached the rail while x=224 and x=227 reached the content viewport —
 * so half the widened hit area was dead and aiming at the element's centre
 * (what `hover()` and `boundingBox()` centre do) landed on the dead side.
 *
 * Cause: `SidebarInset` carries `relative z-10` and is rendered AFTER the rail,
 * and the hit area carried `z-10` too, so equal z-index let DOM order decide.
 * `SidebarRail` now carries `z-20`, which lifts the hit area and the seam
 * together. `railPoint` below still aims at the sidebar edge — that is where a
 * user's cursor is when the resize cursor appears — and the new test at the end
 * of this file asserts the formerly dead half now receives events.
 *
 * The content surface is painted over the `-right-1` half despite the hit
 * area's `z-10`, so only x 220..223 actually reaches the rail. Aiming at the
 * element's centre — what `hover()` and `boundingBox()` centre do — lands on
 * the dead side and the action never arrives. See W5 BUG-11.
 */
const hitArea = (page: Page) => rail(page).locator('span').nth(0);
const seam = (page: Page) => rail(page).locator('span').nth(1);

/**
 * A point inside the half of the hit area that actually receives events: two
 * pixels left of the sidebar's right edge. This is also where a user's cursor
 * sits when the `cursor-col-resize` affordance appears.
 */
async function railPoint(page: Page): Promise<{ x: number; y: number }> {
  const box = await sidebar(page).boundingBox();
  expect(box).not.toBeNull();
  return { x: box!.x + box!.width - 2, y: box!.y + box!.height / 2 };
}

const SIDEBAR_KEYBOARD_STEP = 16; // components/ui/Sidebar.tsx:53

test.describe('App shell — sidebar resize (#5676 AC-4)', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-sidebar-resize-user');
    await dismissWalkthroughIfPresent(page);
  });

  test('ArrowRight and ArrowLeft resize the column in 16px steps', async ({ page }) => {
    const before = await widthOf(page);

    await rail(page).focus();
    await page.keyboard.press('ArrowRight');
    await expect.poll(() => widthOf(page)).toBe(before + SIDEBAR_KEYBOARD_STEP);

    await page.keyboard.press('ArrowLeft');
    await expect.poll(() => widthOf(page)).toBe(before);

    // Two steps in one direction accumulate — a handler that snapped back to a
    // single step would pass the first assertion and fail here.
    await page.keyboard.press('ArrowLeft');
    await page.keyboard.press('ArrowLeft');
    await expect.poll(() => widthOf(page)).toBe(before - 2 * SIDEBAR_KEYBOARD_STEP);
  });

  test('dragging the rail resizes the column', async ({ page }) => {
    const before = await widthOf(page);
    // A real pointer drag, not a synthetic width write: press on the rail, move,
    // release. `handlePointerDown` attaches window-level pointermove/pointerup
    // listeners, so the move has to happen at the page level to be seen.
    const { x: startX, y: startY } = await railPoint(page);
    await page.mouse.move(startX, startY);
    await page.mouse.down();
    await page.mouse.move(startX + 60, startY, { steps: 10 });
    await page.mouse.up();

    await expect.poll(() => widthOf(page)).toBeGreaterThan(before);
  });

  test('a resized width survives a reload', async ({ page }) => {
    // The web lane maps "restart the app" onto a reload
    // (VITE_OPENHUMAN_E2E_RESTART_APP_AS_RELOAD=true), which is the closest this
    // lane gets to #5676's "persists across an app restart". The width lives in
    // the persisted panel-layout store, so a reload exercises the same
    // rehydration path a restart would.
    const before = await widthOf(page);

    await rail(page).focus();
    await page.keyboard.press('ArrowRight');
    await page.keyboard.press('ArrowRight');
    const resized = before + 2 * SIDEBAR_KEYBOARD_STEP;
    await expect.poll(() => widthOf(page)).toBe(resized);

    await page.reload();
    await dismissWalkthroughIfPresent(page);

    await expect.poll(() => widthOf(page), { timeout: 20_000 }).toBe(resized);
    // And specifically NOT back at the default — the assertion above would also
    // hold if `resized` happened to equal the default width.
    expect(resized).not.toBe(before);
  });

  test('the full width of the hit area receives pointer events (#5906)', async ({ page }) => {
    // The regression guard for the stacking fix. Before it, the content card
    // (`SidebarInset`, `relative z-10`, rendered after the rail) painted over
    // the half of the hit area that overhangs it, so `elementFromPoint` there
    // returned the content viewport and a drag started on nothing.
    //
    // Walks the hit area's real box and asserts every sampled x resolves to a
    // node inside the rail. Sampling rather than one point: the failure was
    // exactly that one half worked and the other did not, so a single probe at
    // the wrong offset reports the wrong answer either way.
    const box = await hitArea(page).boundingBox();
    expect(box).not.toBeNull();
    expect(box!.width).toBeGreaterThan(0);

    const y = box!.y + box!.height / 2;
    const offsets = [1, box!.width / 4, box!.width / 2, (box!.width * 3) / 4, box!.width - 1];

    const owners = await page.evaluate(
      ({ xs, yy }) =>
        xs.map(x => {
          const el = document.elementFromPoint(x, yy);
          const railEl = document.querySelector('[data-testid="root-shell-divider"]');
          return {
            x: Math.round(x),
            insideRail: Boolean(railEl && el && (el === railEl || railEl.contains(el))),
            tag: el ? el.tagName : 'null',
          };
        }),
      { xs: offsets.map(o => box!.x + o), yy: y }
    );

    const dead = owners.filter(o => !o.insideRail);
    expect(
      dead,
      `these x positions do not reach the rail: ${dead.map(d => `${d.x} -> ${d.tag}`).join(', ')}`
    ).toEqual([]);
  });

  test('the rail is absent while collapsed — the icon column is fixed, not draggable', async ({
    page,
  }) => {
    // Presence by count, not visibility — see the `hitArea` note above.
    await expect(rail(page)).toHaveCount(1);

    await page.getByRole('button', { name: 'Hide sidebar' }).click();
    await expect(sidebar(page)).toHaveAttribute('data-state', 'collapsed');

    // `RootShellLayout.tsx` renders the rail behind `isOpen &&` precisely so a
    // collapsed column cannot be dragged to an arbitrary width.
    await expect(rail(page)).toHaveCount(0);

    await page.getByTestId('root-shell-reopen').click();
    await expect(sidebar(page)).toHaveAttribute('data-state', 'expanded');
    await expect(rail(page)).toHaveCount(1);
  });
});

test.describe('App shell — the resize seam paints on interaction (#5676 AC-3)', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-sidebar-seam-user');
    await dismissWalkthroughIfPresent(page);
  });

  test('the seam is transparent at rest and paints a visible colour on hover', async ({ page }) => {
    // #5676: "unit tests only assert the class name is applied, never its
    // rendered colour". This reads the colour the engine actually resolved.
    const restColour = await seam(page).evaluate(el => getComputedStyle(el).backgroundColor);

    const pt = await railPoint(page);
    await page.mouse.move(pt.x, pt.y);

    await expect
      .poll(async () => seam(page).evaluate(el => getComputedStyle(el).backgroundColor))
      .not.toBe(restColour);

    const hoverColour = await seam(page).evaluate(el => getComputedStyle(el).backgroundColor);
    // Not transparent, and not fully see-through — the seam has to be visible
    // for the affordance to exist at all.
    expect(hoverColour).not.toBe('rgba(0, 0, 0, 0)');
    expect(hoverColour).not.toMatch(/,\s*0\)$/);
  });

  test('the seam also paints on keyboard focus, not only on hover', async ({ page }) => {
    // The rail is a `role="separator"` the user can Tab to; a seam that only
    // responds to hover leaves keyboard users with no visible target.
    const restColour = await seam(page).evaluate(el => getComputedStyle(el).backgroundColor);

    await rail(page).focus();

    await expect
      .poll(async () => seam(page).evaluate(el => getComputedStyle(el).backgroundColor))
      .not.toBe(restColour);
  });
});
