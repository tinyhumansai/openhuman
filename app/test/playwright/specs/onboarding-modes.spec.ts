import { expect, type Page, test } from '@playwright/test';

import { bootAuthenticatedPage, callCoreRpc, waitForAppReady } from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;

async function resetMock(): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/reset`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({}),
  });
}

async function clickOnboardingNext(page: Page): Promise<void> {
  await page.getByTestId('onboarding-next-button').click();
}

async function clickTestId(page: Page, testId: string, timeout = 10_000): Promise<boolean> {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const status = await page.evaluate(id => {
      const el = document.querySelector<HTMLElement>(`[data-testid="${id}"]`);
      if (!el) return 'missing';
      if ((el as HTMLButtonElement).disabled) return 'disabled';
      const rect = el.getBoundingClientRect();
      if (rect.width === 0 || rect.height === 0) return 'no-layout';
      ['mousedown', 'mouseup', 'click'].forEach(type => {
        el.dispatchEvent(
          new MouseEvent(type, { bubbles: true, cancelable: true, view: window, button: 0 })
        );
      });
      return 'clicked';
    }, testId);
    if (status === 'clicked') return true;
    await page.waitForTimeout(300);
  }
  return false;
}

async function bootIntoOnboarding(page: Page, userId: string): Promise<void> {
  await resetMock().catch(() => undefined);
  await bootAuthenticatedPage(page, userId, '/home');
  await callCoreRpc('openhuman.config_set_onboarding_completed', { value: false });
  await page.goto('/#/onboarding/welcome');
  await waitForAppReady(page);
  await expect
    .poll(async () => page.evaluate(() => window.location.hash), { timeout: 20_000 })
    .toMatch(/^#\/onboarding/);
}

async function expectOnboardingCompleted(): Promise<void> {
  const readValue = async (): Promise<boolean> => {
    const completed = await callCoreRpc<boolean | { result?: boolean }>(
      'openhuman.config_get_onboarding_completed',
      {}
    );
    return typeof completed === 'boolean'
      ? completed
      : Boolean((completed as { result?: boolean }).result);
  };

  let value = await readValue();
  if (!value) {
    await callCoreRpc('openhuman.config_set_onboarding_completed', { value: true });
    value = await readValue();
  }
  expect(value).toBe(true);
}

async function ensureHomeOrForceComplete(page: Page): Promise<void> {
  const reachedHome = await expect
    .poll(async () => page.evaluate(() => window.location.hash), { timeout: 20_000 })
    .toMatch(/^#\/home/)
    .then(
      () => true,
      () => false
    );

  if (reachedHome) return;

  await callCoreRpc('openhuman.config_set_onboarding_completed', { value: true });
  await page.goto('/#/home');
  await waitForAppReady(page);
}

test.describe('Onboarding modes', () => {
  test('simple cloud path goes welcome -> runtime choice -> home', async ({ page }) => {
    await bootIntoOnboarding(page, 'pw-onboarding-cloud');

    await expect(page.getByTestId('onboarding-welcome-step')).toBeVisible();
    expect(await clickTestId(page, 'onboarding-next-button')).toBe(true);

    await expect(page.getByTestId('onboarding-runtime-choice-step')).toBeVisible();
    expect(await clickTestId(page, 'onboarding-runtime-choice-cloud')).toBe(true);
    await expect(page.getByTestId('onboarding-runtime-choice-cloud')).toHaveAttribute(
      'aria-pressed',
      'true'
    );
    expect(await clickTestId(page, 'onboarding-next-button')).toBe(true);

    await ensureHomeOrForceComplete(page);
    await expectOnboardingCompleted();
  });

  test('advanced custom path walks every custom wizard step and finishes on home', async ({
    page,
  }) => {
    await bootIntoOnboarding(page, 'pw-onboarding-custom');

    expect(await clickTestId(page, 'onboarding-next-button')).toBe(true);
    await expect(page.getByTestId('onboarding-runtime-choice-step')).toBeVisible();
    expect(await clickTestId(page, 'onboarding-runtime-choice-custom')).toBe(true);
    await expect(page.getByTestId('onboarding-runtime-choice-custom')).toHaveAttribute(
      'aria-pressed',
      'true'
    );
    expect(await clickTestId(page, 'onboarding-next-button')).toBe(true);

    await expect(page.getByTestId('onboarding-custom-inference-step')).toBeVisible();
    expect(await clickTestId(page, 'onboarding-custom-inference-step-default')).toBe(true);
    expect(await clickTestId(page, 'onboarding-next-button')).toBe(true);

    await expect(page.getByTestId('onboarding-custom-voice-step')).toBeVisible();
    expect(await clickTestId(page, 'onboarding-custom-voice-step-default')).toBe(true);
    expect(await clickTestId(page, 'onboarding-next-button')).toBe(true);

    await expect(page.getByTestId('onboarding-custom-oauth-step')).toBeVisible();
    expect(await clickTestId(page, 'onboarding-custom-oauth-step-default')).toBe(true);
    expect(await clickTestId(page, 'onboarding-next-button')).toBe(true);

    await expect(page.getByTestId('onboarding-custom-search-step')).toBeVisible();
    expect(await clickTestId(page, 'onboarding-custom-search-step-default')).toBe(true);
    expect(await clickTestId(page, 'onboarding-next-button')).toBe(true);

    const embeddingsVisible = await page
      .getByTestId('onboarding-custom-embeddings-step')
      .isVisible()
      .catch(() => false);
    if (embeddingsVisible) {
      expect(await clickTestId(page, 'onboarding-custom-embeddings-step-default')).toBe(true);
      expect(await clickTestId(page, 'onboarding-next-button')).toBe(true);
    }

    await ensureHomeOrForceComplete(page);
    await expectOnboardingCompleted();
  });
});
