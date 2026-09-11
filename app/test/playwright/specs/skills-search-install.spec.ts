import { expect, test } from '@playwright/test';

import {
  bootRuntimeReadyGuestPage,
  dismissWalkthroughIfPresent,
  signInViaCallbackToken,
  waitForAppReady,
} from '../helpers/core-rpc';

/**
 * The Skills explorer's search and install affordances, driven in a real
 * browser.
 *
 * `skills-registry.spec.ts` has a test called "search filters entries by query",
 * but it lives in that file's `Skill registry RPC smoke` describe — it calls
 * `openhuman.skill_registry_search` directly and never touches the UI. Nothing
 * types into the search box, and nothing exercises the install button.
 *
 * The part that only a browser can check is the **debounce**:
 * `SkillsExplorerTab.tsx:22` sets `SEARCH_DEBOUNCE_MS = 300` and `:469-478`
 * restarts the timer on every keystroke, so a burst of typing must produce ONE
 * catalog search rather than one per character. Counting real RPCs against real
 * keystroke timing is not something jsdom reproduces faithfully.
 *
 * NOTE ON SCOPE: this uses `?tab=skills`, never the Composio tab. Opening that
 * one downloads the `tinyconnectors` module from a GitHub release and a failed
 * download is terminal for the core process — see W3-ui-bugs.md §3.
 */

const SEARCH = 'skill-search-input';

async function openSkillsTab(page: import('@playwright/test').Page, userId: string) {
  await bootRuntimeReadyGuestPage(page);
  await signInViaCallbackToken(page, userId);
  await page.evaluate(() => {
    try {
      localStorage.setItem('openhuman:walkthrough_completed', 'true');
      localStorage.removeItem('openhuman:walkthrough_pending');
    } catch {}
    window.location.hash = '/connections?tab=skills';
  });
  await expect
    .poll(() => page.evaluate(() => window.location.hash), { timeout: 15_000 })
    .toContain('tab=skills');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId(SEARCH)).toBeVisible({ timeout: 20_000 });
}

/** Count `skill_registry_search` RPCs, letting everything through untouched. */
async function countSearchRpcs(page: import('@playwright/test').Page) {
  const queries: string[] = [];
  await page.route('**/rpc', async (route, request) => {
    try {
      const body = JSON.parse(request.postData() || '{}');
      if (body.method === 'openhuman.skill_registry_search') {
        queries.push(String(body.params?.query ?? ''));
      }
    } catch {
      /* not JSON — pass it through untouched */
    }
    await route.continue();
  });
  return queries;
}

const searchBox = (page: import('@playwright/test').Page) => page.getByTestId(SEARCH);

test.describe('Skills explorer — the search box debounces', () => {
  test('a burst of typing issues ONE catalog search, not one per keystroke', async ({ page }) => {
    await openSkillsTab(page, 'pw-skills-debounce');
    const queries = await countSearchRpcs(page);

    // Type well inside the 300 ms window. Playwright's default `type` delay is
    // 0, so these land in a few milliseconds of each other.
    await searchBox(page).type('docker', { delay: 20 });

    // Wait past the debounce and let the request settle.
    await expect.poll(() => queries.length, { timeout: 10_000 }).toBeGreaterThan(0);
    await page.waitForTimeout(600);

    expect(
      queries.length,
      `expected one debounced search, got ${queries.length}: ${JSON.stringify(queries)}`
    ).toBe(1);
    // And the one that fired carries the FINAL text, not a prefix.
    expect(queries[0]).toBe('docker');
  });

  test('a pause between words issues a second search', async ({ page }) => {
    // The mirror of the test above: the debounce must delay, not swallow.
    await openSkillsTab(page, 'pw-skills-debounce-two');
    const queries = await countSearchRpcs(page);

    await searchBox(page).type('git', { delay: 20 });
    await expect.poll(() => queries.length, { timeout: 10_000 }).toBe(1);

    await searchBox(page).type('hub', { delay: 20 });
    await expect.poll(() => queries.length, { timeout: 10_000 }).toBe(2);
    expect(queries[1]).toBe('github');
  });
});

test.describe('Skills explorer — typing narrows what is on screen', () => {
  test('a query with no matches leaves no catalog rows', async ({ page }) => {
    await openSkillsTab(page, 'pw-skills-nomatch');

    // Baseline: the catalog has something in it.
    await expect(page.getByRole('row').first()).toBeVisible({ timeout: 20_000 });

    const rows = page.locator('[data-testid^="registry-install-"]');
    await expect(rows.first()).toBeVisible({ timeout: 20_000 });

    await searchBox(page).fill('zzzz-no-such-skill-zzzz');
    // Any install button is a catalog row; none should survive this query.
    await expect(rows).toHaveCount(0, { timeout: 15_000 });

    // NOT asserted: that clearing the box restores the rows. Clearing takes the
    // `!query && !sourceFilter` branch of `fetchCatalog`
    // (SkillsExplorerTab.tsx:517), which calls `skillRegistryApi.browse()` — an
    // UPSTREAM registry fetch. In this lane that is not reliably fast, and an
    // earlier draft asserting it passed alone and failed in a full five-spec
    // run. A flaky assertion is not an improvement on the vacuous one it
    // replaced, so this test pins only the deterministic half: the query
    // empties the list.
  });

  test('the typed text is preserved in the box while results load', async ({ page }) => {
    // A search box that clears itself mid-request loses what the user typed.
    //
    // The search RPC is HELD so the assertion lands while the request is
    // genuinely outstanding. The earlier form typed, slept 800ms and asserted —
    // which proves the value survives a fixed delay, not that it survives a
    // request in flight. By 800ms the search may already have completed, in
    // which case the test says nothing about the "while results load" case its
    // own name claims. Caught in review by `coderabbitai`.
    let releaseSearch!: () => void;
    const searchHeld = new Promise<void>(resolve => {
      releaseSearch = resolve;
    });
    let searchStarted = false;

    await openSkillsTab(page, 'pw-skills-preserve');

    await page.route('**/rpc', async (route, request) => {
      let method = '';
      try {
        method = JSON.parse(request.postData() || '{}').method ?? '';
      } catch {
        /* pass through */
      }
      if (method === 'openhuman.skill_registry_search') {
        searchStarted = true;
        await searchHeld;
      }
      await route.continue();
    });

    await searchBox(page).fill('docker');

    // Wait for the request to actually be in flight, then assert.
    await expect.poll(() => searchStarted, { timeout: 15_000 }).toBe(true);
    await expect(searchBox(page)).toHaveValue('docker');

    releaseSearch();
  });
});

test.describe('Skills explorer — the install button', () => {
  /** Find the first catalog row offering an install. */
  async function firstInstallButton(page: import('@playwright/test').Page) {
    const button = page.locator('[data-testid^="registry-install-"]').first();
    await expect(button).toBeVisible({ timeout: 20_000 });
    return button;
  }

  test('offers Install, and the button is enabled before it is pressed', async ({ page }) => {
    await openSkillsTab(page, 'pw-skills-install-idle');
    const button = await firstInstallButton(page);

    await expect(button).toBeEnabled();
    await expect(button).toHaveText(/install/i);
  });

  test('disables the button and shows Installing while the RPC is in flight', async ({ page }) => {
    // The transition only exists while the request is outstanding, so the
    // install RPC is held open deliberately. This is the state a jsdom test
    // cannot observe without faking timers.
    let release!: () => void;
    const held = new Promise<void>(resolve => {
      release = resolve;
    });

    await page.route('**/rpc', async (route, request) => {
      let method = '';
      try {
        method = JSON.parse(request.postData() || '{}').method ?? '';
      } catch {
        /* pass through */
      }
      if (method === 'openhuman.skill_registry_install') {
        await held;
      }
      await route.continue();
    });

    await openSkillsTab(page, 'pw-skills-install-inflight');
    const button = await firstInstallButton(page);
    await button.click();

    await expect(button).toBeDisabled({ timeout: 10_000 });
    await expect(button).toHaveText(/installing/i);

    release();
  });

  test('a failed install re-enables the button rather than stranding it', async ({ page }) => {
    // If the button stayed disabled on failure the user could not retry, and
    // nothing on screen would say why.
    //
    // The install response is HELD rather than answered immediately. Without
    // that, "enabled and reading Install" is also the button's INITIAL state,
    // so the test passed whether or not the click ever did anything — it could
    // not distinguish recovery from a no-op. Holding lets us prove the round
    // trip: enabled -> (held) disabled/Installing -> released -> enabled again.
    // Caught in review by `coderabbitai`.
    let releaseInstall!: () => void;
    const installHeld = new Promise<void>(resolve => {
      releaseInstall = resolve;
    });
    let installBody: { id?: unknown } = {};

    await page.route('**/rpc', async (route, request) => {
      let body: { method?: string; id?: unknown } = {};
      try {
        body = JSON.parse(request.postData() || '{}');
      } catch {
        /* pass through */
      }
      if (body.method === 'openhuman.skill_registry_install') {
        installBody = body;
        await installHeld;
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            jsonrpc: '2.0',
            id: installBody.id,
            error: { code: -32000, message: 'install refused by the registry' },
          }),
        });
        return;
      }
      await route.continue();
    });

    await openSkillsTab(page, 'pw-skills-install-fail');
    const button = await firstInstallButton(page);
    await expect(button).toBeEnabled();
    await button.click();

    // The pending state must actually be reached before the failure.
    await expect(button).toBeDisabled({ timeout: 15_000 });
    await expect(button).toHaveText(/installing/i);

    releaseInstall();

    await expect(button).toBeEnabled({ timeout: 20_000 });
    await expect(button).toHaveText(/install/i);
  });
});
