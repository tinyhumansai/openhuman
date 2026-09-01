import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

/**
 * Settings navigation, driven in a real browser.
 *
 * # Why this exists alongside `navigation-settings-panels.spec.ts`
 *
 * That spec asserts `#root` innerText is longer than 50 characters and that
 * SOME marker from a list appears. Its markers ('Settings', 'Appearance',
 * 'Notifications', 'Tools'…) are the settings SIDEBAR's own labels, which are
 * rendered on every `/settings/*` route — so each of its cases passes whenever
 * the sidebar renders, whether or not the routed panel does. It cannot
 * distinguish "the panel opened" from "the chrome around it opened".
 *
 * This spec uses two assertions per route, because one is not enough:
 *
 * 1. **The `<h1>`.** Every settings page renders exactly one, and the sidebar
 *    renders none (`SettingsTabbedPage.tsx:70`), so an exact-text assertion
 *    proves a settings page resolved for that URL — which the marker-OR above
 *    cannot.
 *
 *    But it does NOT prove the right panel mounted. `SettingsPanel.tsx:108`
 *    resolves the heading as `title ?? t(findEntryById(currentRoute).titleKey)`,
 *    so for the many panels that pass no explicit `title` the h1 comes from the
 *    ROUTE registry, not the component. I verified this by wiring
 *    `/settings/security` to `<MigrationPanel />` and rebuilding: the heading
 *    still read "Security" and an h1-only spec passed.
 *
 * 2. **A body marker unique to that panel** — a control only that panel
 *    renders. This is what actually pins panel identity, and every marker below
 *    was read out of the live DOM rather than guessed.
 *
 * Both matter: (1) catches a dead or unresolved route, (2) catches a mis-wired
 * one.
 *
 * Selectors come from the route registry, whose `id` is documented as "used as
 * the React key, test id, and route slug"
 * (`settingsRouteRegistry.ts:77`) — the sidebar renders each entry with
 * `data-testid={`settings-nav-${entry.id}`}` (`SettingsSidebar.tsx:50`).
 */

/**
 * Registry id → the route's `<h1>` (from the registry `titleKey`) and a marker
 * that only that panel's BODY renders. Markers were collected from the running
 * app, not inferred from source.
 */
const PANELS = [
  { id: 'appearance', route: 'appearance', heading: 'Appearance', marker: 'font-size-slider' },
  { id: 'privacy', route: 'privacy', heading: 'Privacy', marker: 'privacy-mode-options' },
  { id: 'devices', route: 'devices', heading: 'Devices', marker: null, text: 'Pair iPhone' },
  {
    id: 'security',
    route: 'security',
    heading: 'Security',
    marker: null,
    text: 'Retry keychain detection',
  },
  {
    id: 'notifications',
    route: 'notifications',
    heading: 'Notifications',
    marker: null,
    text: 'Categories',
  },
  {
    id: 'profiles',
    route: 'profiles',
    heading: 'Agent Profiles',
    marker: null,
    text: 'New profile',
  },
  {
    id: 'agent-access',
    route: 'agent-access',
    heading: 'Agent OS access',
    marker: null,
    text: 'View approval history',
  },
  // Sandbox is a desktop-only panel: in the web lane its body is the
  // desktop-only notice, not the Docker fields. Asserting what this build
  // actually renders, rather than what the Tauri build would.
  {
    id: 'sandbox-settings',
    route: 'sandbox-settings',
    heading: 'Sandbox execution',
    marker: null,
    text: 'only available in the desktop app',
  },
  { id: 'about', route: 'about', heading: 'About', marker: 'github-star-cta' },
] as const;

/** The panel-identity assertion: a control only this panel's body renders. */
async function expectPanelBody(page: Page, panel: (typeof PANELS)[number]) {
  if (panel.marker) {
    await expect(page.getByTestId(panel.marker)).toBeVisible({ timeout: 30_000 });
  } else {
    await expect(page.getByText(panel.text!, { exact: false }).first()).toBeVisible({
      timeout: 30_000,
    });
  }
}

const panelHeading = (page: Page) => page.getByRole('heading', { level: 1 });

async function gotoSettings(page: Page, route: string) {
  await page.goto(`/#/settings/${route}`);
  await waitForAppReady(page);
  // Defence in depth: `seedBrowserCoreMode` already sets
  // `openhuman:walkthrough_completed`, which is why this suite passed without
  // it — but if that seed ever changes, an open walkthrough would intercept
  // the sidebar clicks below rather than failing loudly.
  await dismissWalkthroughIfPresent(page);
  // `waitForAppReady` only proves the shell painted; the routed panel mounts a
  // beat later. Wait for the panel's own heading before asserting on it.
  await expect(panelHeading(page)).toBeVisible({ timeout: 30_000 });
}

test.describe('Settings navigation — deep links', () => {
  test.beforeEach(async ({ page }) => {
    // Boot directly to a settings route. The helper's default '/home' hash
    // waits for the chat-shell redirect to settle, which is a slow path this
    // suite never needs — and it exhausted the 60s test budget in beforeEach.
    await bootAuthenticatedPage(page, 'pw-w1-settings-nav', '/settings/appearance');
  });

  for (const panel of PANELS) {
    test(`deep link /settings/${panel.route} mounts the ${panel.id} panel`, async ({ page }) => {
      await gotoSettings(page, panel.route);

      // (1) a settings page resolved for this URL — the sidebar has no h1, so
      // this fails if only the chrome mounted.
      await expect(panelHeading(page)).toHaveText(panel.heading, { timeout: 30_000 });
      // (2) and it is THIS panel, not another one wired to the same route.
      await expectPanelBody(page, panel);
      expect(await page.evaluate(() => window.location.hash)).toBe(`#/settings/${panel.route}`);
    });
  }

  test('the sidebar renders exactly one h1, and it belongs to the panel', async ({ page }) => {
    await gotoSettings(page, 'privacy');

    // If a future layout change gave the sidebar an h1, every assertion above
    // would silently weaken. This is the guard on the discriminator itself.
    await expect(panelHeading(page)).toHaveCount(1);
    await expect(panelHeading(page)).toHaveText('Privacy', { timeout: 30_000 });
  });
});

test.describe('Settings navigation — clicking through the sidebar', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-w1-settings-click', '/settings/appearance');
    await expect(panelHeading(page)).toBeVisible({ timeout: 30_000 });
  });

  test('clicking a sidebar entry swaps the panel and the URL together', async ({ page }) => {
    await expect(panelHeading(page)).toHaveText('Appearance', { timeout: 30_000 });

    await page.getByTestId('settings-nav-privacy').click();

    await expect(panelHeading(page)).toHaveText('Privacy', { timeout: 30_000 });
    await expect(page.getByTestId('privacy-mode-options')).toBeVisible({ timeout: 30_000 });
    await expect
      .poll(async () => page.evaluate(() => window.location.hash))
      .toBe('#/settings/privacy');
  });

  test('walks three panels in sequence without stale content', async ({ page }) => {
    for (const step of ['security', 'devices', 'about'] as const) {
      const expected = PANELS.find(p => p.id === step)!.heading;
      await page.getByTestId(`settings-nav-${step}`).click();
      // `toHaveText` retries, so this also proves the PREVIOUS panel's heading
      // is gone rather than both being present.
      await expect(panelHeading(page)).toHaveText(expected, { timeout: 30_000 });
      await expectPanelBody(page, PANELS.find(p => p.id === step)!);
    }
  });

  test('marks the open panel as the current page for assistive tech', async ({ page }) => {
    await page.getByTestId('settings-nav-privacy').click();
    await expect(panelHeading(page)).toHaveText('Privacy', { timeout: 30_000 });

    const privacyNav = page.getByTestId('settings-nav-privacy');
    const securityNav = page.getByTestId('settings-nav-security');

    // Whatever the mechanism (aria-current or aria-selected), the open entry
    // must be distinguishable from a closed one; a sighted user gets the
    // highlight, and this is the same signal for a screen reader.
    const current = await privacyNav.evaluate(
      el => el.getAttribute('aria-current') ?? el.getAttribute('aria-selected')
    );
    const other = await securityNav.evaluate(
      el => el.getAttribute('aria-current') ?? el.getAttribute('aria-selected')
    );
    expect(current).not.toBeNull();
    expect(current).not.toBe(other);
  });
});

test.describe('Settings navigation — browser history', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-w1-settings-history', '/settings/appearance');
  });

  test('back returns to the previous panel, forward returns again', async ({ page }) => {
    await gotoSettings(page, 'appearance');
    await expect(panelHeading(page)).toHaveText('Appearance', { timeout: 30_000 });

    await page.getByTestId('settings-nav-security').click();
    await expect(panelHeading(page)).toHaveText('Security', { timeout: 30_000 });

    await page.goBack();
    await expect(panelHeading(page)).toHaveText('Appearance', { timeout: 30_000 });
    await expect(page.getByTestId('font-size-slider')).toBeVisible({ timeout: 30_000 });
    await expect
      .poll(async () => page.evaluate(() => window.location.hash))
      .toBe('#/settings/appearance');

    await page.goForward();
    await expect(panelHeading(page)).toHaveText('Security', { timeout: 30_000 });
  });

  test('a reload keeps you on the panel you deep-linked to', async ({ page }) => {
    await gotoSettings(page, 'sandbox-settings');
    await expect(panelHeading(page)).toHaveText('Sandbox execution', { timeout: 30_000 });

    await page.reload();
    await waitForAppReady(page);

    await expect(panelHeading(page)).toHaveText('Sandbox execution', { timeout: 30_000 });
    await expectPanelBody(page, PANELS.find(p => p.id === 'sandbox-settings')!);
  });
});
