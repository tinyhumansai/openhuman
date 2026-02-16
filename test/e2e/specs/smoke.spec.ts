import { waitForApp } from '../helpers/app-helpers';

describe('Smoke tests', () => {
  before(async () => {
    await waitForApp();
  });

  it('app process launched successfully (session created)', async () => {
    // Verify Appium has an active session connected to the app
    const sessionId = browser.sessionId;
    expect(sessionId).toBeDefined();
    expect(typeof sessionId).toBe('string');
    expect(sessionId.length).toBeGreaterThan(0);
  });

  it('app has a menu bar', async () => {
    const menuBar = await browser.$('//XCUIElementTypeMenuBar');
    expect(await menuBar.isExisting()).toBe(true);
  });

  it('app accessibility tree has elements', async () => {
    // Find any element in the app to confirm XCUITest can see it
    const elements = await browser.$$('//*');
    expect(elements.length).toBeGreaterThan(0);
  });
});
