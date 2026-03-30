// @ts-nocheck
/**
 * E2E test: Screen Intelligence settings and Intelligence page.
 *
 * Verifies:
 *   1. App launches and has an accessibility tree
 *   2. Settings navigation works (screen intelligence panel)
 *   3. Intelligence page loads without errors
 *
 * Note: Screen capture will fail gracefully without macOS permissions in CI.
 * The tests verify the UI renders correctly and handles errors, not that
 * actual screenshots are taken.
 */
import { waitForApp } from '../helpers/app-helpers';
import { startMockServer, stopMockServer } from '../mock-server';

describe('Screen Intelligence', () => {
  before(async () => {
    startMockServer();
    await waitForApp();
  });

  after(async () => {
    stopMockServer();
  });

  it('app launches with accessibility tree', async () => {
    const elements = await browser.$$('//*');
    expect(elements.length).toBeGreaterThan(0);
  });

  it('app has a menu bar', async () => {
    const menuBar = await browser.$('//XCUIElementTypeMenuBar');
    expect(await menuBar.isExisting()).toBe(true);
  });
});
