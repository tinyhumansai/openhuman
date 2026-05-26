import { test } from '@playwright/test';

import { bootAuthenticatedPage, waitForAppReady } from '../helpers/core-rpc';

test.describe('Connectivity state differentiation (issue #1527)', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const testSlug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootAuthenticatedPage(page, 'pw-connectivity-diff-' + testSlug, '/home');
  });

  test.skip('shows backend-reconnecting status when backend is unreachable but internet is up', async () => {});

  test.skip('shows reconnecting status after socket is force-disconnected server-side', async () => {});

  test.skip('shows device-offline copy (not backend-only) when window fires offline', async () => {});

  test.skip('status updates to healthy without reinstall after backend recovers from 503', async () => {});

  test.skip('shows core-offline indicator (not device-offline) when internet is up but core is unreachable', async () => {
    // Placeholder until a stop-core command exists in the web/test lane.
  });

  test('baseline app shell is ready in the browser lane', async ({ page }) => {
    await waitForAppReady(page);
  });
});
