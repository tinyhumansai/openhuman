import { expect, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-chat-wallet-flow';

async function resetMock(): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/reset`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({}),
  });
}

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

test.describe('Recovery Phrase wallet setup', () => {
  test('reveals the phrase and confirms the wallet was saved', async ({ page }) => {
    await resetMock();
    await bootAuthenticatedPage(page, USER_ID, '/home');
    await emulateTauriRuntime(page);
    await page.goto('/#/settings/recovery-phrase');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);

    const status = await callCoreRpc<unknown>('openhuman.wallet_status', {});
    expect(JSON.stringify(status)).toMatch(/"configured"\s*:\s*false/);

    await expect(page.getByRole('button', { name: 'Copy to Clipboard' })).toBeVisible();
    await page.getByRole('button', { name: /Reveal recovery phrase/i }).click();
    await page.locator('input[type="checkbox"]').first().check();
    await page.getByRole('button', { name: 'Save Recovery Phrase' }).click();

    await expect(page.getByText(/Recovery phrase saved/i)).toBeVisible();

    const walletState = await callCoreRpc<unknown>('openhuman.test_support_read_workspace_file', {
      rel_path: 'state/wallet-state.json',
      max_bytes: 131_072,
    });
    const content =
      walletState && typeof walletState === 'object' && 'result' in walletState
        ? (walletState as { result?: { content_utf8?: unknown } }).result?.content_utf8
        : undefined;
    expect(typeof content).toBe('string');
    const state = JSON.parse(content as string) as {
      consentGranted?: boolean;
      source?: string;
      accounts?: Array<{ chain?: string }>;
    };
    expect(state.consentGranted).toBe(true);
    expect(state.source).toBe('generated');
    for (const chain of ['evm', 'btc', 'solana', 'tron']) {
      expect(state.accounts?.some(account => account.chain === chain)).toBe(true);
    }
  });
});
