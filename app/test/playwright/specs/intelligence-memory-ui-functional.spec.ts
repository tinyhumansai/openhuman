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

/**
 * Seeds developer mode via `persist:theme` localStorage so the Memory tab
 * (dev-gated in the Activity page since Phase 3) is visible after boot.
 * Must be called via `page.addInitScript` before navigation.
 */
async function seedDeveloperMode(page: Page): Promise<void> {
  await page.addInitScript(() => {
    try {
      const raw = localStorage.getItem('persist:theme');
      const parsed: Record<string, string> = raw ? (JSON.parse(raw) as Record<string, string>) : {};
      parsed.developerMode = JSON.stringify(true);
      localStorage.setItem('persist:theme', JSON.stringify(parsed));
    } catch {}
  });
}

async function openMemory(page: Page): Promise<void> {
  // Phase 3: Memory moved out of /activity entirely — it now lives at
  // /settings/intelligence (the full Intelligence dev surface).  The Memory tab
  // there is not dev-gated (only "council" is), so no developer mode seeding
  // is required for the tab itself; seedDeveloperMode is still called so any
  // code that checks developerMode elsewhere behaves consistently.
  await seedDeveloperMode(page);
  await bootAuthenticatedPage(page, 'pw-intelligence-memory-ui', '/settings/intelligence');
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
