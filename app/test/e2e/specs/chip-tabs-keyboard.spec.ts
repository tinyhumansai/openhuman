import { browser, expect } from '@wdio/globals';

import { waitForApp } from '../helpers/app-helpers';
import { waitForTestId } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

type Tab = 'providers' | 'routing' | 'websites';
const TABS: Tab[] = ['providers', 'routing', 'websites'];

async function expectSelectedAndFocused(selected: Tab): Promise<void> {
  for (const tab of TABS) {
    const element = await waitForTestId(`search-tab-${tab}`);
    await expect(element).toHaveAttribute('aria-selected', String(tab === selected));
    if (tab === selected) await expect(element).toBeFocused();
  }
}

// Registered by app/scripts/e2e-run-all-flows.sh in the navigation suite.
describe('ChipTabs keyboard traversal', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    await resetApp('e2e-chip-tabs-keyboard');
  });

  after(async () => {
    await stopMockServer();
  });

  it('enters on the active tab and selects with arrows, Home, End and wrapping', async () => {
    await navigateViaHash('/settings/search');
    await waitForTestId('search-settings-panel');
    // Start in the middle so each jump changes the controlled selection.
    const middle = await waitForTestId('search-tab-routing');
    await middle.click();
    await expectSelectedAndFocused('routing');

    // Start from the preceding control and enter with a real Tab event. This
    // exercises entry focus in Radix's roving group before arrow-key checks.
    // Search settings load asynchronously from the core. Wait for the switch
    // before focusing it so this setup does not race the panel's loading state.
    const enabledToggle = await waitForTestId('search-enabled-toggle', 15_000);
    await browser.execute((element: HTMLElement) => element.focus(), enabledToggle);
    await expect(enabledToggle).toBeFocused();
    await browser.keys('Tab');
    await expectSelectedAndFocused('routing');

    for (const [key, selected] of [
      ['ArrowRight', 'websites'],
      ['ArrowLeft', 'routing'],
      ['Home', 'providers'],
      ['End', 'websites'],
      ['ArrowRight', 'providers'],
      ['ArrowLeft', 'websites'],
    ] as const) {
      await browser.keys(key);
      await expectSelectedAndFocused(selected);
    }
  });
});
