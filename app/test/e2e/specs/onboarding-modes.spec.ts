// @ts-nocheck
/**
 * E2E coverage for the current onboarding split: self-hosted users configure
 * inference/search/vault, while TinyHumans users complete managed setup and go
 * straight to chat.
 */
import { waitForApp, waitForAppReady, waitForAuthBootstrap } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { waitForWebView, waitForWindowVisible } from '../helpers/element-helpers';
import { triggerAuthLoopbackBypass } from '../helpers/loopback-auth-helpers';
import { resetApp } from '../helpers/reset-app';
import { dismissBootCheckGateIfVisible } from '../helpers/shared-flows';
import {
  resetMockBehavior,
  setMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

async function clickTestId(testId: string, timeout = 10_000): Promise<boolean> {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const clicked = await browser.execute(id => {
      const element = document.querySelector<HTMLElement>(`[data-testid="${id}"]`);
      if (!element || (element as HTMLButtonElement).disabled) return false;
      element.click();
      return true;
    }, testId);
    if (clicked) return true;
    await browser.pause(250);
  }
  return false;
}

async function hasTestId(testId: string, timeout = 15_000): Promise<boolean> {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (
      await browser.execute(id => document.querySelector(`[data-testid="${id}"]`) !== null, testId)
    ) {
      return true;
    }
    await browser.pause(250);
  }
  return false;
}

async function waitForHash(prefix: string, timeout = 30_000): Promise<boolean> {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await browser.execute(value => window.location.hash.startsWith(value), prefix)) return true;
    await browser.pause(250);
  }
  return false;
}

async function onboardingCompleted(): Promise<boolean> {
  const response = await callOpenhumanRpc<boolean | { result: boolean }>(
    'openhuman.config_get_onboarding_completed',
    {}
  );
  if (!response.ok) throw new Error(`Could not read onboarding state: ${response.error}`);
  return typeof response.result === 'boolean' ? response.result : response.result.result;
}

async function setOnboardingCompleted(value: boolean): Promise<void> {
  const response = await callOpenhumanRpc('openhuman.config_set_onboarding_completed', { value });
  if (!response.ok) throw new Error(`Could not set onboarding state: ${response.error}`);
}

describe('Onboarding — self-hosted and managed paths', function () {
  this.timeout(120_000);

  before(async function () {
    this.timeout(90_000);
    await startMockServer();
    resetMockBehavior();
    setMockBehavior('composioConnections', '[]');
    await waitForApp();
    await resetApp('e2e-onboarding-modes', { skipAuth: true, clearAuthSession: true });
  });

  after(async () => {
    resetMockBehavior();
    await stopMockServer();
  });

  it('self-hosted setup runs the three custom steps and supports skipping Search', async function () {
    this.timeout(90_000);
    await setOnboardingCompleted(false);
    expect(await clickTestId('welcome-cta-self')).toBe(true);
    await waitForAppReady(20_000);
    await waitForAuthBootstrap(20_000);

    // The self-hosted CTA creates a local session and routes into the custom
    // wizard. Reapply the flag after identity activation so the gate sees the
    // local user's incomplete state as well.
    await setOnboardingCompleted(false);
    await browser.execute(() => {
      window.location.replace('#/onboarding/custom/inference');
      window.location.reload();
    });
    await waitForAppReady(20_000);

    expect(await hasTestId('onboarding-custom-inference-step')).toBe(true);
    const labels = await browser.execute(() =>
      Array.from(document.querySelectorAll('[data-testid="onboarding-wizard-stepper"] > li')).map(
        item => (item.querySelector('span:last-child')?.textContent ?? '').trim()
      )
    );
    expect(labels).toEqual(['Inference', 'Search', 'Vault']);
    expect(await clickTestId('onboarding-next-button')).toBe(true);

    expect(await hasTestId('onboarding-custom-search-step')).toBe(true);
    expect(await clickTestId('onboarding-search-skip')).toBe(true);
    expect(await hasTestId('onboarding-custom-vault-step')).toBe(true);
    expect(await clickTestId('onboarding-next-button')).toBe(true);

    expect(await waitForHash('#/chat')).toBe(true);
    expect(await onboardingCompleted()).toBe(true);
    expect(await hasTestId('onboarding-custom-voice-step', 500)).toBe(false);
    expect(await hasTestId('onboarding-custom-oauth-step', 500)).toBe(false);
    expect(await hasTestId('onboarding-custom-embeddings-step', 500)).toBe(false);
  });

  it('managed setup completes automatically and lands in chat', async function () {
    this.timeout(90_000);
    const cleared = await callOpenhumanRpc('openhuman.auth_clear_session', {});
    expect(cleared.ok).toBe(true);
    await browser.execute(() => {
      window.location.replace('#/');
      window.location.reload();
    });
    await waitForWindowVisible(20_000);
    await waitForWebView(15_000);
    await waitForAppReady(20_000);
    await dismissBootCheckGateIfVisible(8_000);

    await triggerAuthLoopbackBypass('e2e-onboarding-managed');
    await waitForAppReady(20_000);
    await waitForAuthBootstrap(20_000);

    // A managed account has no local setup to configure. Exercise its welcome
    // route with an incomplete flag and verify that it marks setup complete.
    await setOnboardingCompleted(false);
    await browser.execute(() => {
      window.location.replace('#/onboarding/welcome');
      window.location.reload();
    });
    await waitForAppReady(20_000);
    await waitForAuthBootstrap(20_000);

    const deadline = Date.now() + 20_000;
    let completed = false;
    while (Date.now() < deadline) {
      completed = await onboardingCompleted();
      if (completed) break;
      await browser.pause(250);
    }
    expect(completed).toBe(true);
    expect(await waitForHash('#/chat')).toBe(true);
    expect(await hasTestId('onboarding-layout', 500)).toBe(false);
  });
});
