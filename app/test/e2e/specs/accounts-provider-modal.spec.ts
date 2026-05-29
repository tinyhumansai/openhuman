// @ts-nocheck
import { waitForApp } from '../helpers/app-helpers';
import { supportsExecuteScript } from '../helpers/platform';
import { resetApp } from '../helpers/reset-app';
import {
  clickAddAccountProvider,
  navigateViaHash,
  openAddAccountModal,
  waitForAccountsPage,
  waitForAddAccountModalClosed,
} from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const BASE_PICKER_PROVIDERS = [
  { id: 'whatsapp', label: 'WhatsApp Web' },
  { id: 'wechat', label: 'WeChat Web' },
  { id: 'telegram', label: 'Telegram Web' },
  { id: 'linkedin', label: 'LinkedIn' },
  { id: 'slack', label: 'Slack' },
  { id: 'discord', label: 'Discord' },
];

const HIDDEN_ACCOUNT_PROVIDERS = ['google-meet', 'zoom'];
const DEV_PICKER_PROVIDER = { id: 'browserscan', label: 'BrowserScan (dev)' };

function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[AccountsProviderModalE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[AccountsProviderModalE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

async function getVisiblePickerProviderIds(): Promise<string[]> {
  return browser.execute(() =>
    Array.from(document.querySelectorAll('[data-testid^="add-account-provider-"]'))
      .map(el => el.getAttribute('data-testid')?.replace('add-account-provider-', ''))
      .filter(Boolean)
      .sort()
  );
}

async function providerTileExists(providerId: string): Promise<boolean> {
  return browser.execute(
    id => Boolean(document.querySelector(`[data-testid="add-account-provider-${id}"]`)),
    providerId
  );
}

async function registeredProviders(): Promise<string[]> {
  return browser.execute(() => {
    const winAny = window as unknown as { __OPENHUMAN_STORE__?: { getState: () => unknown } };
    const state = winAny.__OPENHUMAN_STORE__?.getState() as
      | { accounts?: { accounts?: Record<string, { provider?: string }> } }
      | undefined;
    const accounts = state?.accounts?.accounts ?? {};
    return Object.values(accounts)
      .map(a => a.provider)
      .filter(Boolean)
      .sort();
  });
}

describe('Accounts provider picker contract', () => {
  before(async function beforeSuite() {
    if (!supportsExecuteScript()) {
      stepLog('Skipping suite on Mac2 — provider picker needs DOM test ids');
      this.skip();
    }

    await startMockServer();
    await waitForApp();
    await resetApp('e2e-accounts-provider-modal');
  });

  after(async () => {
    stepLog('stopping mock server');
    await stopMockServer();
  });

  it('shows every exposed account provider and keeps hidden providers out of the picker', async () => {
    stepLog('navigating to account surface');
    await navigateViaHash('/chat');
    await waitForAccountsPage();

    stepLog('opening Add Account modal');
    await openAddAccountModal();

    for (const provider of BASE_PICKER_PROVIDERS) {
      const tile = await browser.$(`[data-testid="add-account-provider-${provider.id}"]`);
      await tile.waitForDisplayed({ timeout: 10_000 });
      expect(await tile.getText()).toContain(provider.label);
    }

    for (const providerId of HIDDEN_ACCOUNT_PROVIDERS) {
      expect(await providerTileExists(providerId)).toBe(false);
    }

    const visibleProviderIds = await getVisiblePickerProviderIds();
    stepLog('visible provider ids', visibleProviderIds);
    for (const provider of BASE_PICKER_PROVIDERS) {
      expect(visibleProviderIds).toContain(provider.id);
    }
    expect(visibleProviderIds).not.toContain('google-meet');
    expect(visibleProviderIds).not.toContain('zoom');

    await browser.execute(() => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    });
    await waitForAddAccountModalClosed();
  });

  it('registers each visible provider through the real picker interaction', async () => {
    await navigateViaHash('/chat');
    await waitForAccountsPage();
    await openAddAccountModal();

    const visibleProviderIds = await getVisiblePickerProviderIds();
    const providersToRegister = BASE_PICKER_PROVIDERS.filter(provider =>
      visibleProviderIds.includes(provider.id)
    );
    if (visibleProviderIds.includes(DEV_PICKER_PROVIDER.id)) {
      providersToRegister.push(DEV_PICKER_PROVIDER);
    }

    stepLog(
      'providers to register',
      providersToRegister.map(provider => provider.id)
    );
    await browser.execute(() => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    });
    await waitForAddAccountModalClosed();

    for (const provider of providersToRegister) {
      stepLog(`registering ${provider.id}`);
      await navigateViaHash('/chat');
      await waitForAccountsPage();
      await openAddAccountModal();
      await clickAddAccountProvider(provider.id);
      await waitForAddAccountModalClosed();

      const registered = await browser.waitUntil(
        async () => {
          const providers = await registeredProviders();
          return providers.includes(provider.id);
        },
        {
          timeout: 5_000,
          timeoutMsg: `Redux accounts slice never recorded provider ${provider.id}`,
        }
      );
      expect(registered).toBe(true);
    }

    const providers = await registeredProviders();
    for (const provider of providersToRegister) {
      expect(providers).toContain(provider.id);
    }
    expect(providers).not.toContain('google-meet');
    expect(providers).not.toContain('zoom');
  });
});
