// @ts-nocheck
/**
 * E2E coverage for the current first-launch choices, local setup, managed
 * sign-in, and logout flow.
 */
import { waitForApp, waitForAppReady, waitForAuthBootstrap } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { waitForText, waitForWebView, waitForWindowVisible } from '../helpers/element-helpers';
import { triggerAuthLoopbackBypass } from '../helpers/loopback-auth-helpers';
import { resetApp } from '../helpers/reset-app';
import {
  dismissBootCheckGateIfVisible,
  logoutViaSettings,
  waitForRequest,
} from '../helpers/shared-flows';
import {
  clearRequestLog,
  getRequestLog,
  resetMockBehavior,
  setMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

const LOG = '[WelcomeFlow]';

async function clickTestId(testId: string, timeout = 15_000): Promise<boolean> {
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

async function setOnboardingCompleted(value: boolean): Promise<void> {
  const result = await callOpenhumanRpc('openhuman.config_set_onboarding_completed', { value });
  if (!result.ok) throw new Error(`Could not set onboarding state: ${result.error}`);
}

async function getOnboardingCompleted(): Promise<boolean> {
  const result = await callOpenhumanRpc<boolean | { result: boolean }>(
    'openhuman.config_get_onboarding_completed',
    {}
  );
  if (!result.ok) throw new Error(`Could not read onboarding state: ${result.error}`);
  return typeof result.result === 'boolean' ? result.result : result.result.result;
}

describe('Welcome → local setup → managed login → logout', function () {
  this.timeout(180_000);

  before(async function () {
    this.timeout(90_000);
    await startMockServer();
    resetMockBehavior();
    setMockBehavior('composioConnections', '[]');
    await waitForApp();
    await resetApp('e2e-welcome-login', { skipAuth: true, clearAuthSession: true });
    clearRequestLog();
  });

  after(async () => {
    resetMockBehavior();
    await stopMockServer();
  });

  it('shows both current welcome choices and provider sign-in buttons', async function () {
    this.timeout(60_000);
    await waitForWindowVisible(20_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    expect(await waitForText('Welcome to OpenHuman', 15_000)).toBeTruthy();
    expect(await hasTestId('welcome-card-self')).toBe(true);
    expect(await hasTestId('welcome-cta-self')).toBe(true);
    expect(await hasTestId('welcome-cta-tinyhumans')).toBe(true);
    const providersVisible = await browser.execute(() => {
      const buttons = Array.from(document.querySelectorAll('button'));
      return buttons.some(button =>
        /Google|GitHub|Twitter|Discord/i.test(
          button.getAttribute('aria-label') || button.textContent || ''
        )
      );
    });
    expect(providersVisible).toBe(true);
  });

  it('self-hosted choice opens and completes the custom wizard', async function () {
    this.timeout(90_000);
    await setOnboardingCompleted(false);
    expect(await clickTestId('welcome-cta-self')).toBe(true);
    await waitForAppReady(20_000);
    await waitForAuthBootstrap(20_000);
    await setOnboardingCompleted(false);
    await browser.execute(() => {
      window.location.replace('#/onboarding/custom/inference');
      window.location.reload();
    });
    await waitForAppReady(20_000);

    expect(await hasTestId('onboarding-custom-inference-step')).toBe(true);
    expect(await clickTestId('onboarding-next-button')).toBe(true);
    expect(await hasTestId('onboarding-custom-search-step')).toBe(true);
    expect(await clickTestId('onboarding-search-skip')).toBe(true);
    expect(await hasTestId('onboarding-custom-vault-step')).toBe(true);
    expect(await clickTestId('onboarding-next-button')).toBe(true);
    expect(await waitForHash('#/chat')).toBe(true);
    expect(await getOnboardingCompleted()).toBe(true);
  });

  it('managed sign-in completes its setup and lands in chat', async function () {
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

    clearRequestLog();
    await triggerAuthLoopbackBypass('e2e-welcome-managed-user');
    await waitForWindowVisible(20_000);
    await waitForWebView(15_000);
    await waitForAppReady(20_000);
    await waitForAuthBootstrap(20_000);

    expect(await waitForRequest(getRequestLog, 'GET', '/auth/me', 20_000)).toBeTruthy();
    expect(await waitForHash('#/chat')).toBe(true);
    expect(await getOnboardingCompleted()).toBe(true);
  });

  it('logout returns the user to the current Welcome choices', async function () {
    this.timeout(60_000);
    await logoutViaSettings(LOG);
    expect(await waitForText('Welcome to OpenHuman', 15_000)).toBeTruthy();
    expect(await hasTestId('welcome-cta-self')).toBe(true);
    expect(await hasTestId('welcome-cta-tinyhumans')).toBe(true);
  });
});
