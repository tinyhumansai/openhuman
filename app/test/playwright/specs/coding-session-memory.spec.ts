import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, waitForAppReady } from '../helpers/core-rpc';

test.describe('Coding-agent session memory', () => {
  test('shows Codex and Claude Code discovery on the Brain sources page', async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-coding-session-memory', '/brain?tab=sources');
    await waitForAppReady(page);

    const card = page.getByTestId('coding-sessions-card');
    await expect(card).toBeVisible({ timeout: 20_000 });
    await expect(card).toContainText('Coding-agent sessions');
    await expect(page.getByTestId('coding-session-source-claude_code')).toBeVisible();
    await expect(page.getByTestId('coding-session-source-codex')).toBeVisible();
    await expect(page.getByTestId('coding-sessions-ingest')).toBeVisible();
  });
});
