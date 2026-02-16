import { waitForApp } from '../helpers/app-helpers';

describe('Tauri app integration', () => {
  before(async () => {
    await waitForApp();
  });

  it('app has a menu bar (macOS native integration)', async () => {
    const menuBar = await browser.$('//XCUIElementTypeMenuBar');
    expect(await menuBar.isExisting()).toBe(true);
  });

  it('app can take a screenshot (XCUITest bridge works)', async () => {
    const screenshot = await browser.takeScreenshot();
    expect(screenshot).toBeTruthy();
    expect(screenshot.length).toBeGreaterThan(100);
  });
});
