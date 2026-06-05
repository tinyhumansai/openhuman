import { expect, type Page, test } from '@playwright/test';
import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
  bootAuthenticatedPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

type MemorySource = { id: string; kind: string; label: string; enabled: boolean };

async function openMemory(page: Page): Promise<void> {
  await bootAuthenticatedPage(page, 'pw-intelligence-memory-ui', '/intelligence');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  const memoryTab = page.getByRole('tab', { name: /^Memory$/ });
  if (await memoryTab.isVisible().catch(() => false)) {
    await memoryTab.click();
  }
  await expect(page.getByTestId('memory-workspace')).toBeVisible({ timeout: 20_000 });
}

async function addFolderSource(label: string): Promise<string> {
  const root = mkdtempSync(join(tmpdir(), 'openhuman-pw-memory-'));
  mkdirSync(join(root, 'notes'), { recursive: true });
  writeFileSync(join(root, 'notes', 'project.md'), '# Project\n\nPlaywright memory source canary.');
  await callCoreRpc('openhuman.memory_sources_add', {
    kind: 'folder',
    label,
    enabled: true,
    path: root,
    glob: '**/*.md',
  });
  return root;
}

test.describe('Intelligence memory UI', () => {
  test('source row sync, toggle, remove, graph mode, and reset confirmations work', async ({
    page,
  }) => {
    const label = `Playwright Memory Source ${Date.now()}`;
    await openMemory(page);
    await addFolderSource(label);
    await page.reload();
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);
    const memoryTab = page.getByRole('tab', { name: /^Memory$/ });
    if (await memoryTab.isVisible().catch(() => false)) {
      await memoryTab.click();
    }
    await expect(page.getByTestId('memory-workspace')).toBeVisible({ timeout: 20_000 });

    const row = page.getByTestId('memory-source-row-folder').filter({ hasText: label });
    await expect(row).toBeVisible({ timeout: 20_000 });

    await row.getByTestId('memory-source-sync-folder').click();
    await expect(row).toContainText(/Sync|Syncing/);

    await row.getByTitle('Disable').click();
    await expect(row.getByTitle('Enable')).toBeVisible({ timeout: 15_000 });

    await page.getByTestId('memory-graph-mode-contacts').click();
    await expect(page.getByTestId('memory-graph-mode-contacts')).toHaveAttribute(
      'aria-selected',
      'true'
    );
    await page.getByTestId('memory-graph-mode-tree').click();
    await expect(page.getByTestId('memory-graph-mode-tree')).toHaveAttribute(
      'aria-selected',
      'true'
    );

    page.once('dialog', dialog => dialog.dismiss());
    await page.getByTestId('memory-wipe-all').click();
    await expect(page.getByTestId('memory-wipe-all')).toBeEnabled();

    page.once('dialog', dialog => dialog.dismiss());
    await page.getByTestId('memory-reset-tree').click();
    await expect(page.getByTestId('memory-reset-tree')).toBeEnabled();

    await row.getByTitle('Remove').click();
    await expect(row).toHaveCount(0);
  });
});
