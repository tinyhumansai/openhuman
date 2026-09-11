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
 * Command palette, keyboard-only — the arrow-key path.
 *
 * `command-palette.spec.ts` already covers open-by-shortcut, Enter-to-execute,
 * Escape-to-dismiss and the seed action list. Two things it does not do:
 *
 *   1. It never presses an arrow key. It types a query and hits Enter
 *      immediately, so the highlight only ever sits on cmdk's default first
 *      item. Every ArrowDown / ArrowUp / wrap-around behaviour is unexercised —
 *      and a palette you cannot move the selection in is a palette you can only
 *      use by typing an exact match.
 *   2. It uses `input.fill()`, which sets the value in one shot. cmdk filters
 *      per keystroke, so `pressSequentially` is the path a user actually takes.
 *
 * Selection marker is cmdk's own `aria-selected` on `[cmdk-item]`, and each
 * item's `value` is the action id (`CommandPalette.tsx:92`).
 */

const PALETTE_INPUT = 'input[cmdk-input]';
const ITEM = '[cmdk-item]';

/**
 * Where each seed navigation action ACTUALLY lands — the handler's target in
 * `lib/commands/globalActions.ts:119-170` followed through every redirect.
 *
 * Traced from source rather than assumed, which caught two errors in my first
 * version of this map (#5887, CodeRabbit):
 *
 *   nav.intelligence -> `/settings/intelligence`, which is itself
 *     `<Navigate to="/brain">` (settingsRouteElements.tsx:184), so it lands on
 *     `#/brain` — NOT `#/settings/intelligence` as I first wrote.
 *   nav.settings -> `/settings`, whose index is `SettingsIndexRedirect`; at the
 *     >=768px viewport Playwright runs, that is `<Navigate to="/settings/account">`
 *     (SettingsIndexRedirect.tsx:15-18). So `#/settings/account`.
 *
 * Every pattern is end-anchored. A loose `/^#\/settings/` also matches
 * `#/settings/notifications`, which is another mapped action's destination — so
 * the test could pass while Enter ran the wrong item, which is the exact
 * regression it exists to catch (#5887, CodeRabbit).
 */
const DESTINATIONS: Record<string, RegExp> = {
  'nav.home': /^#\/chat(?:[/?]|$)/, // /home -> /chat
  'nav.chat': /^#\/chat(?:[/?]|$)/,
  'nav.intelligence': /^#\/brain(?:[?]|$)/, // /settings/intelligence -> /brain
  'nav.skills': /^#\/connections(?:[?]|$)/,
  'nav.activity': /^#\/settings\/notifications(?:[?]|$)/, // /activity -> here
  'nav.settings': /^#\/settings\/account(?:[?]|$)/, // /settings index -> account
};

const shortcut = () => (process.platform === 'darwin' ? 'Meta+K' : 'Control+K');

async function openPalette(page: Page) {
  await page.keyboard.press(shortcut());
  await expect(page.locator(PALETTE_INPUT)).toBeVisible();
}

/** The action id of the currently highlighted item. */
async function selectedId(page: Page): Promise<string | null> {
  return page.evaluate(() => {
    const el = document.querySelector('[cmdk-item][aria-selected="true"]');
    return el?.getAttribute('data-value') ?? null;
  });
}

async function visibleIds(page: Page): Promise<string[]> {
  return page.evaluate(() =>
    Array.from(document.querySelectorAll('[cmdk-item]')).map(
      el => el.getAttribute('data-value') ?? ''
    )
  );
}

const hash = (page: Page) => page.evaluate(() => window.location.hash);

test.describe('Command palette — keyboard-only selection', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-palette-keyboard-user');
    await dismissWalkthroughIfPresent(page);
  });

  test('ArrowDown and ArrowUp move the highlight between items', async ({ page }) => {
    await openPalette(page);

    const ids = await visibleIds(page);
    expect(ids.length).toBeGreaterThan(2);

    // cmdk highlights the first item on open.
    await expect.poll(() => selectedId(page)).toBe(ids[0]);

    await page.keyboard.press('ArrowDown');
    await expect.poll(() => selectedId(page)).toBe(ids[1]);

    await page.keyboard.press('ArrowDown');
    await expect.poll(() => selectedId(page)).toBe(ids[2]);

    await page.keyboard.press('ArrowUp');
    await expect.poll(() => selectedId(page)).toBe(ids[1]);

    // Exactly one item is ever highlighted.
    await expect(page.locator(`${ITEM}[aria-selected="true"]`)).toHaveCount(1);
  });

  test('Enter runs the arrow-selected action, not the first one', async ({ page }) => {
    // This is the assertion that distinguishes a working arrow key from a
    // decorative one: if ArrowDown moved the highlight visually but Enter still
    // fired item 0, the test must fail.
    //
    // Start somewhere NO mapped action lands, so "the route changed" carries
    // information. `/brain` would not do — it is nav.intelligence's landing.
    await page.goto('/#/notifications');
    await openPalette(page);

    const ids = await visibleIds(page);
    const first = ids[0];
    const target = ids[1];
    expect(target).toBeTruthy();

    const firstDest = DESTINATIONS[first];
    const targetDest = DESTINATIONS[target];
    expect(targetDest, `no known destination for '${target}' — extend DESTINATIONS`).toBeTruthy();

    // If the top two actions happen to land in the same place, this fixture
    // cannot tell them apart and a pass would mean nothing. Fail loudly and
    // say so, rather than reporting a green that discriminates nothing.
    expect(
      firstDest?.source,
      `items 0 ('${first}') and 1 ('${target}') share a destination; ` +
        'this test cannot detect the regression it names — pick a different fixture'
    ).not.toBe(targetDest.source);

    await page.keyboard.press('ArrowDown');
    await expect.poll(() => selectedId(page)).toBe(target);

    await page.keyboard.press('Enter');
    await expect(page.locator(PALETTE_INPUT)).toHaveCount(0);

    // The exact destination of the SELECTED action — not merely "some route".
    await expect.poll(() => hash(page)).toMatch(targetDest);
  });

  test('typing filters per keystroke and the highlight follows the surviving item', async ({
    page,
  }) => {
    await openPalette(page);
    const before = (await visibleIds(page)).length;

    // Real keystrokes, not `fill` — cmdk re-filters on each one.
    await page.locator(PALETTE_INPUT).pressSequentially('connect', { delay: 30 });

    await expect.poll(async () => (await visibleIds(page)).length).toBeLessThan(before);
    await expect.poll(async () => (await visibleIds(page)).length).toBeGreaterThan(0);

    // Whatever survived the filter, something is highlighted — an empty
    // highlight after filtering means Enter does nothing and the palette looks
    // broken.
    await expect.poll(() => selectedId(page)).not.toBeNull();
    const survivor = await selectedId(page);
    expect(await visibleIds(page)).toContain(survivor);
  });

  test('a query matching nothing shows the empty state and Enter is inert', async ({ page }) => {
    await openPalette(page);
    await page.locator(PALETTE_INPUT).pressSequentially('zzzzznotacommand', { delay: 20 });

    await expect.poll(async () => (await visibleIds(page)).length).toBe(0);
    await expect.poll(() => selectedId(page)).toBeNull();

    // Enter on an empty result set must not close the palette on a phantom
    // selection or navigate somewhere arbitrary.
    const before = await hash(page);
    await page.keyboard.press('Enter');
    await expect(page.locator(PALETTE_INPUT)).toBeVisible();
    expect(await hash(page)).toBe(before);
  });

  test('the whole flow is reachable without a mouse', async ({ page }) => {
    // Open, filter, move, execute — no click() anywhere in this test.
    await openPalette(page);
    await page.locator(PALETTE_INPUT).pressSequentially('set', { delay: 30 });
    await expect.poll(async () => (await visibleIds(page)).length).toBeGreaterThan(0);
    await page.keyboard.press('Enter');

    await expect(page.locator(PALETTE_INPUT)).toHaveCount(0);
    await expect.poll(() => hash(page)).toMatch(/^#\/settings/);
  });
});
