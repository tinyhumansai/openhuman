import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

/**
 * RecoveryPhrasePanel — the replace gate, in a real browser.
 *
 * # What is already covered, and what is not
 *
 * `settings-account-preferences.spec.ts` drives generate → confirm → save and
 * asserts the wallet ends up configured. Its helper *clicks through* the
 * replace gate to reach generate mode, but nothing asserts the gate itself:
 * that a configured wallet cannot reach the generate flow without passing it,
 * that the warning is shown, and — the part that matters — that backing out
 * leaves the existing key material untouched.
 *
 * This is the highest-stakes flow in the app: confirming replace destroys the
 * user's only copy of their wallet. A cancel that silently rotated the key
 * would be unrecoverable, and no jsdom test can prove it did not, because the
 * proof is in the core's `wallet_status`, not in the DOM.
 *
 * Each test therefore reads the wallet's real state over RPC before and after
 * the interaction, and compares.
 */

/**
 * Make `isTauri()` true for the page. The crypto settings surface is
 * desktop-gated, so without this `/settings/recovery-phrase` never mounts in
 * the web lane and the router falls through to chat. Copied per-spec, as the
 * other specs that need it do (`settings-advanced-config.spec.ts:9`,
 * `chat-harness-wallet-flow.spec.ts:71`) — `helpers/` is shared and off-limits.
 */
async function emulateTauriRuntime(page: Page): Promise<void> {
  await page.evaluate(() => {
    const win = window as typeof window & {
      isTauri?: boolean;
      __TAURI_INTERNALS__?: { invoke?: (cmd: string, args?: unknown) => Promise<unknown> };
    };
    win.isTauri = true;
    win.__TAURI_INTERNALS__ = win.__TAURI_INTERNALS__ ?? {};
    win.__TAURI_INTERNALS__.invoke = win.__TAURI_INTERNALS__.invoke ?? (async () => null);
  });
}

interface WalletStatus {
  configured?: boolean;
  accounts?: { address?: string }[];
}

async function walletState(): Promise<{ configured: boolean; addresses: string[] }> {
  const status = await callCoreRpc<{ result?: WalletStatus }>('openhuman.wallet_status', {});
  return {
    configured: Boolean(status.result?.configured),
    addresses: (status.result?.accounts ?? [])
      .map(a => a?.address ?? '')
      .filter(Boolean)
      .sort(),
  };
}

const replaceButton = (page: Page) => page.getByRole('button', { name: 'Replace wallet' });
const confirmReplace = (page: Page) =>
  page.getByRole('button', { name: 'I understand, replace my wallet' });
const importInstead = (page: Page) =>
  page.getByRole('button', { name: 'I already have a recovery phrase' });
const cancelButton = (page: Page) => page.getByRole('button', { name: 'Cancel' });
const copyButton = (page: Page) => page.getByRole('button', { name: 'Copy to Clipboard' });

async function gotoRecoveryPhrase(page: Page) {
  await page.goto('/#/settings/recovery-phrase');
  await waitForAppReady(page);
  // A walkthrough overlay intercepts clicks on a fresh profile; the working
  // account spec dismisses it the same way before touching this panel.
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Recovery phrase', {
    timeout: 30_000,
  });
}

/**
 * Leave the account with a configured wallet and the panel in `view` mode.
 *
 * The wallet's existence is checked over RPC rather than by reading the panel's
 * mode: `view` vs `generate` is derived from an async status fetch, so a
 * UI-first check races the first render on a fresh account.
 */
async function ensureConfiguredWallet(page: Page) {
  if (!(await walletState()).configured) {
    await gotoRecoveryPhrase(page);
    await expect(copyButton(page)).toBeVisible({ timeout: 30_000 });
    await page.locator('#mnemonic-confirm-checkbox').check();
    await page.getByRole('button', { name: 'Save Recovery Phrase' }).click();

    // Wait on the CORE, not the DOM: the saved-note is transient and the panel
    // re-renders as the wallet lands.
    await expect.poll(async () => (await walletState()).configured, { timeout: 45_000 }).toBe(true);
  }

  // Re-enter so the panel reads the now-configured status from a clean mount.
  // Navigating to the SAME hash is a no-op for the router, so the panel would
  // keep its post-save state and never re-read the wallet — hop via another
  // settings route to force a real remount.
  await page.goto('/#/settings/privacy');
  await waitForAppReady(page);
  await gotoRecoveryPhrase(page);
  await expect(replaceButton(page)).toBeVisible({ timeout: 30_000 });
}

test.describe('Recovery phrase — the replace gate', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-w1-recovery');
    await emulateTauriRuntime(page);
    await ensureConfiguredWallet(page);
  });

  test('a configured wallet shows no phrase and offers no direct regenerate', async ({ page }) => {
    // View mode must not display the mnemonic until it is explicitly revealed,
    // and the destructive action must sit behind the gate rather than being a
    // one-click affordance.
    await expect(replaceButton(page)).toBeVisible();
    await expect(confirmReplace(page)).toHaveCount(0);
    await expect(copyButton(page)).toHaveCount(0);
  });

  test('clicking Replace wallet shows the gate, not the new phrase', async ({ page }) => {
    await replaceButton(page).click();

    await expect(confirmReplace(page)).toBeVisible();
    await expect(importInstead(page)).toBeVisible();
    await expect(cancelButton(page)).toBeVisible();
    // The gate must not have already produced a replacement phrase behind it.
    await expect(copyButton(page)).toHaveCount(0);
  });

  test('Cancel returns to view mode and leaves the wallet byte-identical', async ({ page }) => {
    const before = await walletState();
    expect(before.configured).toBe(true);
    expect(before.addresses.length).toBeGreaterThan(0);

    await replaceButton(page).click();
    await expect(confirmReplace(page)).toBeVisible();

    await cancelButton(page).click();

    // Back to view mode...
    await expect(replaceButton(page)).toBeVisible();
    await expect(confirmReplace(page)).toHaveCount(0);

    // ...and the core still holds exactly the same key material. This is the
    // assertion the DOM cannot make on its own.
    const after = await walletState();
    expect(after).toEqual(before);
  });

  test('navigating away from the open gate does not replace the wallet', async ({ page }) => {
    const before = await walletState();

    await replaceButton(page).click();
    await expect(confirmReplace(page)).toBeVisible();

    // Abandoning the flow mid-gate is the likeliest real-world exit, and must
    // be as safe as pressing Cancel.
    await page.goto('/#/settings/privacy');
    await waitForAppReady(page);
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Privacy');

    expect(await walletState()).toEqual(before);
  });

  test('re-entering the panel after Cancel starts from view mode, not the gate', async ({
    page,
  }) => {
    await replaceButton(page).click();
    await expect(confirmReplace(page)).toBeVisible();
    await cancelButton(page).click();
    await expect(replaceButton(page)).toBeVisible();

    await page.goto('/#/settings/privacy');
    await waitForAppReady(page);
    await gotoRecoveryPhrase(page);

    // A gate left open across navigation would be a loaded gun on return.
    await expect(replaceButton(page)).toBeVisible({ timeout: 20_000 });
    await expect(confirmReplace(page)).toHaveCount(0);
  });

  test('"I already have a recovery phrase" opens import, and importing nothing changes nothing', async ({
    page,
  }) => {
    const before = await walletState();

    await replaceButton(page).click();
    await importInstead(page).click();

    // Import mode: word slots, and no generated phrase to copy. The slots are
    // addressed by their aria-label — they carry no id
    // (`RecoveryPhraseImportMode.tsx:88`).
    await expect(page.getByLabel('Recovery phrase word 1', { exact: true })).toBeVisible({
      timeout: 30_000,
    });
    await expect(copyButton(page)).toHaveCount(0);

    // Leaving without completing the import must not touch the wallet.
    await page.goto('/#/settings/privacy');
    await waitForAppReady(page);

    expect(await walletState()).toEqual(before);
  });
});
