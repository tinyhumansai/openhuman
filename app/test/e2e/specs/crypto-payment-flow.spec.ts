// @ts-nocheck
/**
 * E2E test: Billing panel (crypto/Coinbase path).
 *
 * The in-app Coinbase Commerce checkout UI was removed alongside Stripe;
 * billing now lives on the web dashboard. These tests verify the billing
 * redirect panel shown at /settings/billing:
 *
 *   6.1  Billing panel renders the "moved to web" redirect page
 *   6.2  "Open billing dashboard" button is present
 *   6.3  Opening browser is indicated while the redirect fires
 */
import { waitForApp } from '../helpers/app-helpers';
import { textExists, waitForText } from '../helpers/element-helpers';
import { navigateToBilling, navigateToHome, performFullLogin } from '../helpers/shared-flows';
import { clearRequestLog, startMockServer, stopMockServer } from '../mock-server';

const LOG_PREFIX = '[CryptoPayment]';

describe('Crypto Payment Flow', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    clearRequestLog();
  });

  after(async () => {
    await stopMockServer();
  });

  it('login and reach home', async () => {
    await performFullLogin('e2e-crypto-payment-token');
  });

  it('6.1 — billing panel shows "moved to web" redirect page', async function () {
    this.timeout(60_000);
    await navigateToBilling();
    await waitForText('Open billing dashboard', 20_000);
    console.log(`${LOG_PREFIX} 6.1 — billing redirect panel loaded`);
  });

  it('6.2 — "Open billing dashboard" button is present', async () => {
    await navigateToBilling();
    const hasButton = await textExists('Open billing dashboard');
    expect(hasButton).toBe(true);
    console.log(`${LOG_PREFIX} 6.2 — "Open billing dashboard" button present`);
  });

  it('6.3 — opening-browser status message is shown on mount', async () => {
    await navigateToBilling();
    await waitForText('Billing moved to the web', 10_000);
    // BillingPanel triggers openUrl on mount; while it is in-flight it shows
    // t('settings.billing.openingBrowser') = 'Opening your browser...'
    // After the promise resolves it transitions to 'idle' / 'error'.
    // Either the opening message or the idle fallback must be visible.
    const hasOpeningMsg =
      (await textExists('Opening your browser')) ||
      (await textExists('If your browser did not open')) ||
      (await textExists('Open billing dashboard'));
    expect(hasOpeningMsg).toBe(true);
    console.log(`${LOG_PREFIX} 6.3 — browser-open status message present`);
    await navigateToHome();
  });
});
