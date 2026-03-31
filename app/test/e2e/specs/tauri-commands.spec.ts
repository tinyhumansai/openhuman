import { waitForApp } from '../helpers/app-helpers';
import { hasAppChrome } from '../helpers/element-helpers';

describe('Tauri app integration', () => {
  before(async () => {
    await waitForApp();
  });

  it('app chrome is visible (native integration)', async () => {
    expect(await hasAppChrome()).toBe(true);
  });

  it('app can take a screenshot (driver bridge works)', async () => {
    const screenshot = await browser.takeScreenshot();
    expect(screenshot).toBeTruthy();
    expect(screenshot.length).toBeGreaterThan(100);
  });
});
