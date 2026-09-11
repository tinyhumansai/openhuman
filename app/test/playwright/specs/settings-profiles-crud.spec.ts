import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

/**
 * Agent profiles — the full create / activate / delete journey, verified
 * against the core.
 *
 * My jsdom specs (`ProfileEditorPage.payload.test.tsx`,
 * `ProfilesPanel.actions.test.tsx`) assert the payload the panel BUILDS and the
 * errors it renders, with the API mocked. What neither can show is that the
 * profile actually reaches `openhuman.profiles_upsert`, comes back in
 * `profiles_list`, survives a reload, and disappears on delete. No spec in any
 * lane does that today.
 *
 * Each test therefore reads the core's own profile list over RPC and compares.
 */

interface Profile {
  id?: string;
  name?: string;
  description?: string;
  builtIn?: boolean;
}

/**
 * NOTE the shape: `profiles_list` returns `{ activeProfileId, profiles }`
 * DIRECTLY — unlike `wallet_status`, whose payload is nested under a second
 * `result`. Reading `res.result?.profiles` here silently yields `[]`, so every
 * core comparison passes vacuously or fails for the wrong reason. Verified
 * against the live RPC.
 */
async function coreProfiles(): Promise<{ profiles: Profile[]; activeId?: string }> {
  const res = await callCoreRpc<{ profiles?: Profile[]; activeProfileId?: string }>(
    'openhuman.profiles_list',
    {}
  );
  return { profiles: res.profiles ?? [], activeId: res.activeProfileId };
}

const coreIds = async () =>
  (await coreProfiles()).profiles
    .map(p => p.id)
    .filter(Boolean)
    .sort();

async function openProfiles(page: Page) {
  await page.goto('/#/settings/profiles');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Agent Profiles', {
    timeout: 30_000,
  });
}

/** The list row for a profile, addressed by its visible name. */
const row = (page: Page, name: string) => page.locator('li').filter({ hasText: name }).first();

/** Remove a profile directly, so a test's own leftovers cannot leak forward. */
async function deleteFromCore(id: string) {
  // The param is snake_case `profile_id` (agentProfilesApi.ts:43) — `profileId`
  // is accepted by the transport and ignored, so the delete silently no-ops.
  await callCoreRpc('openhuman.profiles_delete', { profile_id: id }).catch(() => {});
}

test.describe('Agent profiles — create', () => {
  // These tests share one hard-coded profile id against a single core, so they
  // must not interleave: a parallel worker's afterEach delete can land between
  // another test's create and its assertion. Serialising the describe is the
  // fix that keeps the id stable and readable (tinysweeper, test-isolation).
  test.describe.configure({ mode: 'serial' });
  const NAME = 'W1 Browser Profile';
  const ID = 'w1-browser-profile';

  test.beforeEach(async ({ page }) => {
    await deleteFromCore(ID);
    await bootAuthenticatedPage(page, 'pw-w1-profiles', '/settings/profiles');
    await openProfiles(page);
  });

  test.afterEach(async () => {
    await deleteFromCore(ID);
  });

  test('creating a profile persists it to the core and lists it', async ({ page }) => {
    expect(await coreIds()).not.toContain(ID);

    await page.getByRole('button', { name: 'New profile' }).click();
    await expect(page.getByLabel('Name', { exact: true })).toBeVisible({ timeout: 30_000 });

    await page.getByLabel('Name', { exact: true }).click();
    await page.keyboard.type(NAME);
    await expect(page.getByLabel('ID', { exact: true })).toHaveValue(ID);

    await page.getByRole('button', { name: 'Create' }).click();

    // Back on the list, and the core holds it.
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Agent Profiles', {
      timeout: 30_000,
    });
    await expect.poll(coreIds, { timeout: 20_000 }).toContain(ID);
    await expect(row(page, NAME)).toBeVisible({ timeout: 30_000 });
  });

  test('a created profile is custom, not built-in, and offers Delete', async ({ page }) => {
    await page.getByRole('button', { name: 'New profile' }).click();
    await page.getByLabel('Name', { exact: true }).click();
    await page.keyboard.type(NAME);
    await page.getByRole('button', { name: 'Create' }).click();
    await expect.poll(coreIds, { timeout: 20_000 }).toContain(ID);

    const created = (await coreProfiles()).profiles.find(p => p.id === ID);
    expect(created?.builtIn ?? false).toBe(false);

    // Built-ins hide Delete; a custom profile must offer it.
    await expect(row(page, NAME).getByText('Delete')).toBeVisible({ timeout: 30_000 });
  });

  test('the created profile survives a reload', async ({ page }) => {
    await page.getByRole('button', { name: 'New profile' }).click();
    await page.getByLabel('Name', { exact: true }).click();
    await page.keyboard.type(NAME);
    await page.getByRole('button', { name: 'Create' }).click();
    await expect.poll(coreIds, { timeout: 20_000 }).toContain(ID);

    await page.reload();
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);

    await expect(row(page, NAME)).toBeVisible({ timeout: 30_000 });
  });
});

test.describe('Agent profiles — activate and delete', () => {
  // These tests share one hard-coded profile id against a single core, so they
  // must not interleave: a parallel worker's afterEach delete can land between
  // another test's create and its assertion. Serialising the describe is the
  // fix that keeps the id stable and readable (tinysweeper, test-isolation).
  test.describe.configure({ mode: 'serial' });
  const NAME = 'W1 Lifecycle Profile';
  const ID = 'w1-lifecycle-profile';

  test.beforeEach(async ({ page }) => {
    await deleteFromCore(ID);
    await callCoreRpc('openhuman.profiles_upsert', {
      profile: {
        id: ID,
        name: NAME,
        description: 'Created over RPC so the test starts from a known list.',
        agentId: 'orchestrator',
        builtIn: false,
        includeAgentConversations: true,
      },
    }).catch(() => {});
    await bootAuthenticatedPage(page, 'pw-w1-profiles-life', '/settings/profiles');
    await openProfiles(page);
  });

  test.afterEach(async () => {
    await deleteFromCore(ID);
  });

  test('Set as active moves the active profile in the core', async ({ page }) => {
    const before = (await coreProfiles()).activeId;
    expect(before).not.toBe(ID);

    await row(page, NAME).getByText('Set as active').click();

    await expect.poll(async () => (await coreProfiles()).activeId, { timeout: 20_000 }).toBe(ID);
    // The row stops offering "Set as active" once it IS active.
    await expect(row(page, NAME).getByText('Set as active')).toHaveCount(0, { timeout: 30_000 });
  });

  test('deleting removes it from the core and from the list', async ({ page }) => {
    page.once('dialog', d => void d.accept());

    await expect(row(page, NAME)).toBeVisible({ timeout: 30_000 });
    await row(page, NAME).getByText('Delete').click();

    await expect.poll(coreIds, { timeout: 20_000 }).not.toContain(ID);
    await expect(page.getByText(NAME)).toHaveCount(0, { timeout: 30_000 });
  });

  test('dismissing the delete confirm keeps the profile', async ({ page }) => {
    page.once('dialog', d => void d.dismiss());

    await row(page, NAME).getByText('Delete').click();

    // Nothing should have been asked of the core, and the row stays.
    await expect(row(page, NAME)).toBeVisible();
    expect(await coreIds()).toContain(ID);
  });

  test('Edit opens the editor with the profile hydrated from the core', async ({ page }) => {
    await row(page, NAME).getByText('Edit').click();

    await expect(page.getByLabel('Name', { exact: true })).toHaveValue(NAME, { timeout: 30_000 });
    // Edit mode renders the id as a non-editable `<code>`, not a read-only input:
    // `ProfileEditorPage.tsx:224-229` branches on `isCreate` and only the create
    // branch renders a `SettingsTextField` with an `aria-label`. So there is no
    // labelled control to find and the count is 0, not 1 (tinysweeper flagged this
    // as possibly a hidden-but-present field; it is genuinely absent).
    await expect(page.getByLabel('ID', { exact: true })).toHaveCount(0);
  });
});

/**
 * Failure paths (#5944 / #5900).
 *
 * Every test above is a happy path, which is how `[object Object]` shipped:
 * `dispatch(thunk).unwrap()` rejects with Redux Toolkit's `SerializedError` — a
 * plain object, never an `Error` — so the old `err instanceof Error` guard took
 * its `String(err)` branch and stringified an object. The defect was even pinned
 * as *expected* in a jsdom test before #5944 inverted it.
 *
 * These drive the real panels against a real core with only the one failing
 * method stubbed, so the whole chain the fix lives on runs:
 * `core RPC error → CoreRpcError → thunk rejection → SerializedError →
 * errorMessage() → rendered text`. `coreRpcClient.ts:855-859` puts the JSON-RPC
 * `error.message` on `CoreRpcError` verbatim, so the string asserted here is the
 * one the backend sent.
 *
 * Each asserts the message IS shown *and* that `[object Object]` is NOT — the
 * first fails on a regression, the second names the specific defect.
 */
test.describe('Agent profiles — a failing action shows the reason', () => {
  test.describe.configure({ mode: 'serial' });
  const NAME = 'W6 Failure Profile';
  const ID = 'w6-failure-profile';

  /**
   * Fail exactly one RPC method with a JSON-RPC error, passing everything else
   * through to the real core.
   *
   * The message is deliberately bland: `classifyRpcError` re-routes anything
   * that looks like an auth, timeout or not-found failure, and a reclassified
   * error would take a different path through the UI than the one under test.
   */
  const failMethod = async (page: Page, method: string, message: string) => {
    await page.route('**/rpc', async (route, request) => {
      const body = JSON.parse(request.postData() || '{}');
      if (body.method === method) {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ jsonrpc: '2.0', id: body.id, error: { code: -32000, message } }),
        });
        return;
      }
      await route.continue();
    });
  };

  test.afterEach(async () => {
    await deleteFromCore(ID);
  });

  test('a failing save shows the backend reason, not [object Object]', async ({ page }) => {
    const REASON = 'the core declined to store this profile';
    await deleteFromCore(ID);
    await bootAuthenticatedPage(page, 'pw-w6-profile-fail', '/settings/profiles');
    await openProfiles(page);
    await failMethod(page, 'openhuman.profiles_upsert', REASON);

    await page.getByRole('button', { name: 'New profile' }).click();
    await expect(page.getByLabel('Name', { exact: true })).toBeVisible({ timeout: 30_000 });
    await page.getByLabel('Name', { exact: true }).click();
    await page.keyboard.type(NAME);
    await page.getByRole('button', { name: 'Create' }).click();

    const alert = page.getByRole('alert');
    await expect(alert).toContainText(REASON, { timeout: 30_000 });
    await expect(alert).not.toContainText('[object Object]');

    // The editor must stay open on failure — navigating back to the list would
    // discard what the user typed on an error they can act on.
    await expect(page.getByLabel('Name', { exact: true })).toHaveValue(NAME);
  });

  test('a failing Set as active shows the backend reason, not [object Object]', async ({
    page,
  }) => {
    const REASON = 'the core declined to switch profile';
    await deleteFromCore(ID);
    await callCoreRpc('openhuman.profiles_upsert', {
      profile: {
        id: ID,
        name: NAME,
        description: 'Seeded over RPC so the list starts from a known state.',
        agentId: 'orchestrator',
        builtIn: false,
        includeAgentConversations: true,
      },
    }).catch(() => {});
    await bootAuthenticatedPage(page, 'pw-w6-profile-fail-select', '/settings/profiles');
    await openProfiles(page);
    await failMethod(page, 'openhuman.profiles_select', REASON);

    await row(page, NAME).getByText('Set as active').click();

    // ProfilesPanel renders `actionError` as a plain styled <p>, not an Alert,
    // so this is located by its text rather than by role.
    await expect(page.getByText(REASON)).toBeVisible({ timeout: 30_000 });
    await expect(page.getByText('[object Object]')).toHaveCount(0);
  });

  test('a failing delete shows the backend reason, not [object Object]', async ({ page }) => {
    const REASON = 'the core declined to remove this profile';
    await deleteFromCore(ID);
    await callCoreRpc('openhuman.profiles_upsert', {
      profile: {
        id: ID,
        name: NAME,
        description: 'Seeded over RPC so the list starts from a known state.',
        agentId: 'orchestrator',
        builtIn: false,
        includeAgentConversations: true,
      },
    }).catch(() => {});
    await bootAuthenticatedPage(page, 'pw-w6-profile-fail-delete', '/settings/profiles');
    await openProfiles(page);
    await failMethod(page, 'openhuman.profiles_delete', REASON);

    page.once('dialog', d => void d.accept());
    await row(page, NAME).getByText('Delete').click();

    await expect(page.getByText(REASON)).toBeVisible({ timeout: 30_000 });
    await expect(page.getByText('[object Object]')).toHaveCount(0);

    // The delete failed, so the profile must still be there.
    await expect(row(page, NAME)).toBeVisible();
  });
});
