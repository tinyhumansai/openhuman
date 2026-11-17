import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, callCoreRpc, waitForAppReady } from '../helpers/core-rpc';

function unwrapResult(value: unknown): unknown {
  let current = value;
  for (let depth = 0; depth < 3; depth += 1) {
    if (!current || typeof current !== 'object' || !('result' in current)) return current;
    current = (current as { result: unknown }).result;
  }
  return current;
}

async function listCronJobs(): Promise<Array<{ id?: string; name?: string; enabled?: boolean }>> {
  const jobs = unwrapResult(await callCoreRpc<unknown>('openhuman.cron_list', {}));
  expect(Array.isArray(jobs), 'cron_list returns job rows').toBe(true);
  return jobs as Array<{ id?: string; name?: string; enabled?: boolean }>;
}

async function openCronJobsPanel(page: import('@playwright/test').Page): Promise<void> {
  await page.goto('/#/settings/cron-jobs');
  await waitForAppReady(page);
  // Panel title dropped in the PanelPage migration; the panel test id and its
  // Scheduled Jobs section confirm it mounted.
  await expect(page.getByTestId('cron-jobs-panel')).toBeVisible();
  await expect(page.getByText('Scheduled Jobs').first()).toBeVisible();
}

test.describe('Cron jobs settings panel', () => {
  test.beforeEach(async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-cron-jobs-flow', '/home');
  });

  test('chat surface is reachable after login', async ({ page }) => {
    // Home folded into the unified chat surface: post-login landing is /chat.
    await waitForAppReady(page);
    await expect(page.getByTestId('chat-message-input')).toBeVisible();
  });

  test('a cron row toggles, persists after refresh, and can be removed', async ({ page }) => {
    const jobsBefore = await listCronJobs();
    let morning = jobsBefore.find(job => job.name === 'morning_briefing');
    if (!morning) {
      const created = unwrapResult(
        await callCoreRpc<unknown>('openhuman.cron_create', {
          name: 'morning_briefing',
          schedule: '0 8 * * *',
          enabled: true,
        })
      );
      morning = created as { id?: string; name?: string; enabled?: boolean };
    }
    const morningId = morning.id;
    expect(morningId, 'morning_briefing has a generated id').toBeTruthy();
    if (morning.enabled === false) {
      await callCoreRpc('openhuman.cron_update', { job_id: morningId, patch: { enabled: true } });
    }

    await openCronJobsPanel(page);
    await page.getByTestId('cron-refresh').click();
    const row = page.getByTestId(`cron-job-row-${morningId}`);
    const toggle = page.getByTestId(`cron-job-toggle-${morningId}`);

    await expect(row).toBeVisible();
    await expect(toggle).toHaveAttribute('aria-label', 'Pause');
    await toggle.click();
    await expect(toggle).toHaveAttribute('aria-label', 'Resume');
    await page.getByTestId('cron-refresh').click();
    await expect(toggle).toHaveAttribute('aria-label', 'Resume');
    await toggle.click();
    await expect(toggle).toHaveAttribute('aria-label', 'Pause');

    const removable = unwrapResult(
      await callCoreRpc<unknown>('openhuman.cron_add', {
        name: 'e2e_remove_me',
        schedule: { kind: 'cron', expr: '0 8 * * *' },
        job_type: 'agent',
        prompt: 'Disposable cron removal fixture',
      })
    ) as { id?: string };
    const removableId = removable.id;
    expect(removableId, 'cron_add returns a generated id').toBeTruthy();
    await callCoreRpc('openhuman.cron_update', { job_id: removableId, patch: { enabled: false } });
    await page.getByTestId('cron-refresh').click();
    const removableRow = page.getByTestId(`cron-job-row-${removableId}`);
    await expect(removableRow).toBeVisible();
    await page.getByTestId(`cron-job-remove-${removableId}`).click();
    await expect(removableRow).toHaveCount(0);
    expect((await listCronJobs()).some(job => job.id === removableId)).toBe(false);
  });

  test('refresh action is visible in the cron jobs panel', async ({ page }) => {
    await openCronJobsPanel(page);
    await expect(page.getByRole('button', { name: 'Refresh Cron Jobs' })).toBeVisible();
  });
});
