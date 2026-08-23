import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

/**
 * Browser-level verification for issue #5676 ("Visually verify sidebar
 * icon-collapse mode on desktop").
 *
 * jsdom cannot composite, has no window chrome, and does not paint CSS custom
 * properties, so the unit suite can only assert class names and inline widths.
 * These specs run against real Chromium: computed styles here are *resolved*
 * colours, so a wrong token (`bg-line` instead of `bg-line-chrome`) fails even
 * though the class name looks plausible.
 *
 * What this covers, per the issue's acceptance criteria:
 *
 * 1. Collapsed-state contract — `collapsible="icon"` keeps a real, non-zero
 *    {@link ICON_WIDTH_PX}px column mounted across repeated toggles and window
 *    resizes (the DOM half of the punch-through question; the native-layer half
 *    stays manual, see docs/qa/sidebar-icon-collapse.md).
 * 2. Traffic-light clearance — the collapsed rail reserves its draggable strip
 *    above every rail item (geometry only; clickability is macOS-manual).
 * 3. Seam colour — the rail indicator resolves to the live `--line-chrome`
 *    token on hover and focus, in light and dark, across four theme presets,
 *    and never to the plain `--line` token it replaced.
 * 4. Drag resize — pointer drag resizes the column, arrow keys step it by
 *    {@link KEYBOARD_STEP_PX}, and the width persists across an app restart
 *    (`VITE_OPENHUMAN_E2E_RESTART_APP_AS_RELOAD` makes that a page reload in
 *    the web E2E build).
 *
 * Set `PW_SIDEBAR_SHOTS=1` to drop evidence screenshots into
 * `test-results/sidebar-shots/`.
 */

const SIDEBAR = '[data-testid="root-shell-sidebar"]';
const RAIL = '[data-testid="root-shell-divider"]';
const REOPEN = '[data-testid="root-shell-reopen"]';
const COLLAPSE_BUTTON = '[data-analytics-id="sidebar-header-collapse"]';

/** Mirrors `SIDEBAR_ICON_WIDTH` in `app/src/components/ui/Sidebar.tsx`. */
const ICON_WIDTH_PX = 56;
/** Mirrors `SIDEBAR_KEYBOARD_STEP` in `app/src/components/ui/Sidebar.tsx`. */
const KEYBOARD_STEP_PX = 16;
/** Mirrors `WINDOW_DRAG_BAR_HEIGHT` in `app/src/components/layout/shell/WindowDragBar.tsx`. */
const DRAG_STRIP_HEIGHT_PX = 28;
/** Mirrors `SIDEBAR_MIN_WIDTH` / `SIDEBAR_MAX_WIDTH` in `app/src/components/ui/Sidebar.tsx`. */
const MIN_WIDTH_PX = 188;
const MAX_WIDTH_PX = 420;

const SHOTS_DIR = 'test-results/sidebar-shots';

interface ThemeCombo {
  name: string;
  familyId: string;
  variant: 'light' | 'dark';
}

/**
 * Four of the five built-in families, in both modes, including a light variant
 * of a dark-default family (Matrix). The issue asks for at least two presets;
 * these five cover classic (both), ocean, matrix-light and HAL 9000.
 */
const THEME_COMBOS: ThemeCombo[] = [
  { name: 'classic-light', familyId: 'classic', variant: 'light' },
  { name: 'classic-dark', familyId: 'classic', variant: 'dark' },
  { name: 'ocean-dark', familyId: 'ocean', variant: 'dark' },
  { name: 'matrix-light', familyId: 'matrix', variant: 'light' },
  { name: 'hal9000-dark', familyId: 'hal9000', variant: 'dark' },
];

function tripleToRgb(triple: string): string {
  const [r, g, b] = triple.trim().split(/\s+/).map(Number);
  return `rgb(${r}, ${g}, ${b})`;
}

/**
 * Seed the persisted theme before any app script boots. redux-persist stores
 * every slice field as an individually stringified JSON value under
 * `persist:<key>`, and requires `_persist` to treat the blob as valid.
 */
async function seedTheme(page: Page, combo: ThemeCombo): Promise<void> {
  await page.addInitScript(({ familyId, variant }) => {
    try {
      const raw = localStorage.getItem('persist:theme');
      const parsed: Record<string, string> = raw ? JSON.parse(raw) : {};
      parsed.activeThemeId = JSON.stringify(familyId);
      parsed.themeVariant = JSON.stringify(variant);
      parsed.mode = JSON.stringify(variant);
      if (!parsed._persist) {
        parsed._persist = JSON.stringify({ version: -1, rehydrated: true });
      }
      localStorage.setItem('persist:theme', JSON.stringify(parsed));
    } catch {
      /* best effort: fall back to the default Classic light theme */
    }
  }, combo);
}

async function bootOnChat(page: Page, combo?: ThemeCombo): Promise<void> {
  if (combo) await seedTheme(page, combo);
  await bootAuthenticatedPage(page, 'pw-sidebar-shell', '/chat');
  await dismissWalkthroughIfPresent(page);
}

async function sidebarWidthPx(page: Page): Promise<number> {
  return page.locator(SIDEBAR).evaluate(el => el.getBoundingClientRect().width);
}

/**
 * `sidebarWidth` as read back out of the per-user persisted layout blob
 * (`${userId}:persist:layout`), i.e. what an app restart would rehydrate.
 */
async function persistedSidebarWidth(page: Page): Promise<number | null> {
  return page.evaluate(() => {
    const uid = localStorage.getItem('OPENHUMAN_ACTIVE_USER_ID');
    const raw = uid ? localStorage.getItem(`${uid}:persist:layout`) : null;
    if (!raw) return null;
    try {
      const parsed = JSON.parse(raw) as { panels?: string };
      const panels = typeof parsed.panels === 'string' ? JSON.parse(parsed.panels) : parsed.panels;
      const panel = panels?.['app-shell'] as { sidebarWidth?: number } | undefined;
      return typeof panel?.sidebarWidth === 'number' ? panel.sidebarWidth : null;
    } catch {
      return null;
    }
  });
}

/** Resolved `R G B` triple of a token custom property on `<html>`. */
async function tokenTriple(page: Page, token: string): Promise<string> {
  return page.evaluate(
    name => getComputedStyle(document.documentElement).getPropertyValue(name).trim(),
    token
  );
}

test.describe('Sidebar icon-collapse verification (#5676)', () => {
  test('collapsed state keeps a real icon-width column mounted through toggles and resizes', async ({
    page,
  }) => {
    await bootOnChat(page);

    const sidebar = page.locator(SIDEBAR);
    await expect(sidebar).toHaveAttribute('data-collapsible', 'icon');
    await expect(sidebar).toHaveAttribute('data-state', 'expanded');

    if (process.env.PW_SIDEBAR_SHOTS) {
      await page.screenshot({ path: `${SHOTS_DIR}/expanded.png`, fullPage: false });
    }

    // Collapse via the header control the same way a user would.
    await page.click(COLLAPSE_BUTTON);
    await expect(sidebar).toHaveAttribute('data-state', 'collapsed');

    // The column must stay mounted and hold the fixed icon width.
    await expect(sidebar).toBeVisible();
    expect(await sidebarWidthPx(page)).toBe(ICON_WIDTH_PX);
    // The resize seam belongs to the expanded state only.
    await expect(page.locator(RAIL)).toHaveCount(0);
    // The reopen affordance and the icon nav rail are present inside the column.
    await expect(page.locator(REOPEN)).toBeVisible();
    await expect(sidebar.locator('nav').first()).toBeVisible();

    if (process.env.PW_SIDEBAR_SHOTS) {
      await page.screenshot({ path: `${SHOTS_DIR}/collapsed.png`, fullPage: false });
    }

    // Toggle repeatedly: the column must survive every transition mounted.
    for (let i = 0; i < 3; i += 1) {
      await page.locator(REOPEN).click();
      await expect(sidebar).toHaveAttribute('data-state', 'expanded');
      await page.click(COLLAPSE_BUTTON);
      await expect(sidebar).toHaveAttribute('data-state', 'collapsed');
      expect(await sidebarWidthPx(page)).toBe(ICON_WIDTH_PX);
    }

    // Resize the window while collapsed: the rail must keep its width.
    await page.setViewportSize({ width: 1000, height: 700 });
    expect(await sidebarWidthPx(page)).toBe(ICON_WIDTH_PX);
    await page.setViewportSize({ width: 1280, height: 720 });

    // Reopen once more so later assertions see the expanded shell.
    await page.locator(REOPEN).click();
    await expect(sidebar).toHaveAttribute('data-state', 'expanded');
  });

  test('collapsed rail reserves a traffic-light-clearing drag strip above its items', async ({
    page,
  }) => {
    await bootOnChat(page);
    await page.click(COLLAPSE_BUTTON);
    await expect(page.locator(REOPEN)).toBeVisible();

    const sidebarBox = await page.locator(SIDEBAR).boundingBox();
    const strip = page.locator(`${SIDEBAR} [data-tauri-drag-region]`).first();
    const stripBox = await strip.boundingBox();
    if (!sidebarBox || !stripBox) throw new Error('collapsed rail geometry missing');

    // Full-width strip pinned to the top of the column, one title-bar tall.
    expect(stripBox.height).toBe(DRAG_STRIP_HEIGHT_PX);
    expect(stripBox.x).toBeCloseTo(sidebarBox.x, 0);
    expect(stripBox.y).toBeCloseTo(sidebarBox.y, 0);
    expect(stripBox.width).toBeGreaterThanOrEqual(ICON_WIDTH_PX - 1);

    // Every interactive item starts below the strip, clear of the lights zone.
    const reopenBox = await page.locator(REOPEN).boundingBox();
    expect(reopenBox?.y ?? 0).toBeGreaterThanOrEqual(stripBox.y + stripBox.height - 1);
  });

  for (const combo of THEME_COMBOS) {
    test(`seam indicator resolves to line-chrome, never line: ${combo.name}`, async ({ page }) => {
      await bootOnChat(page, combo);

      // The seeded family/variant actually applied.
      const isDark = await page.locator('html').evaluate(el => el.classList.contains('dark'));
      expect(isDark).toBe(combo.variant === 'dark');
      await expect(page.locator(SIDEBAR)).toHaveAttribute('data-state', 'expanded');

      const chromeRgb = tripleToRgb(await tokenTriple(page, '--line-chrome'));
      const plainLineRgb = tripleToRgb(await tokenTriple(page, '--line'));
      // The two tokens must actually differ, or this assertion proves nothing.
      expect(chromeRgb).not.toBe(plainLineRgb);

      const rail = page.locator(RAIL);
      const indicator = rail.locator('span').last();

      // At rest the seam paints nothing.
      const restColor = await indicator.evaluate(el => getComputedStyle(el).backgroundColor);
      expect(restColor).toBe('rgba(0, 0, 0, 0)');

      // Hover: resolved background settles exactly on --line-chrome. The
      // indicator carries `transition-colors`, so poll rather than race it.
      await rail.hover();
      await expect
        .poll(async () => indicator.evaluate(el => getComputedStyle(el).backgroundColor))
        .toBe(chromeRgb);

      // Focus: same verdict through the group-focus variant.
      await rail.focus();
      await expect
        .poll(async () => indicator.evaluate(el => getComputedStyle(el).backgroundColor))
        .toBe(chromeRgb);

      if (process.env.PW_SIDEBAR_SHOTS) {
        await page.screenshot({
          path: `${SHOTS_DIR}/seam-hover-${combo.name}.png`,
          fullPage: false,
        });
      }
    });
  }

  test('drag resizes the column, arrow keys step it, and the width survives a restart', async ({
    page,
  }) => {
    await bootOnChat(page);

    const before = Math.round(await sidebarWidthPx(page));
    expect(before).toBeGreaterThanOrEqual(MIN_WIDTH_PX);
    expect(before).toBeLessThanOrEqual(MAX_WIDTH_PX);

    // Pointer drag the rail +60px, like a mouse user would.
    const rail = page.locator(RAIL);
    const box = await rail.boundingBox();
    if (!box) throw new Error('resize rail not mounted');
    const grabY = box.y + box.height / 2;
    await page.mouse.move(box.x + box.width / 2, grabY);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2 + 60, grabY, { steps: 10 });
    await page.mouse.up();

    const dragged = Math.min(MAX_WIDTH_PX, Math.max(MIN_WIDTH_PX, before + 60));
    // The gesture commits once on release; poll instead of racing React.
    await expect.poll(() => sidebarWidthPx(page), { timeout: 5_000 }).toBeCloseTo(dragged, 0);

    // Arrow-key resize still applies, in both directions.
    await rail.focus();
    await page.keyboard.press('ArrowRight');
    await expect
      .poll(() => sidebarWidthPx(page), { timeout: 5_000 })
      .toBeCloseTo(Math.min(MAX_WIDTH_PX, dragged + KEYBOARD_STEP_PX), 0);
    await page.keyboard.press('ArrowLeft');
    await expect.poll(() => sidebarWidthPx(page), { timeout: 5_000 }).toBeCloseTo(dragged, 0);

    // The committed width reaches the per-user persisted blob (redux-persist
    // throttles its writes, so wait for the payload rather than racing it).
    await expect.poll(() => persistedSidebarWidth(page), { timeout: 5_000 }).toBe(dragged);

    // App restart: in the web E2E build the sanctioned analogue of relaunching
    // the desktop app is a full reload, which rehydrates that same blob.
    await page.reload();
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);

    await expect(page.locator(SIDEBAR)).toHaveAttribute('data-state', 'expanded');
    await expect.poll(() => sidebarWidthPx(page), { timeout: 5_000 }).toBeCloseTo(dragged, 0);
  });
});
