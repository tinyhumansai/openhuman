// @ts-nocheck
/**
 * Smoke spec — proves the unified Appium/CEF harness can:
 *
 *   1. Attach to the running app and produce a live WebDriver session.
 *   2. Drive the app from a clean slate through `resetApp(...)`:
 *      sidecar wipe → renderer reload → auth deep-link → onboarding walk.
 *   3. Land on `/home` with rendered React content (NOT a blank shell, NOT
 *      stuck behind BootCheckGate / onboarding / the login screen).
 *
 * Every other spec assumes this works — so when CI is red, look here first.
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { hasAppChrome } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { waitForHomePage } from '../helpers/shared-flows';

const USER_ID = 'e2e-smoke';

describe('Smoke', () => {
  before(async () => {
    await waitForApp();
    await resetApp(USER_ID);
  });

  it('has a live WebDriver session', async () => {
    const sessionId = browser.sessionId;
    expect(sessionId).toBeDefined();
    expect(typeof sessionId).toBe('string');
    expect(sessionId.length).toBeGreaterThan(0);
  });

  it('shows app chrome (window is mapped & visible)', async () => {
    expect(await hasAppChrome()).toBe(true);
  });

  it('renders a non-empty DOM in the main webview', async () => {
    const elements = await browser.$$('//*');
    expect(elements.length).toBeGreaterThan(0);
  });

  it('lands on /home with rendered content after auth + onboarding', async () => {
    await waitForAppReady(10_000);
    const hash = await browser.execute(() => window.location.hash);
    expect(hash).toMatch(/^#\/home/);

    const homeText = await waitForHomePage(15_000);
    expect(homeText).toBeTruthy();
  });
});
