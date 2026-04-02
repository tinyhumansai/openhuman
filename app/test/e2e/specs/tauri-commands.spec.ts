import { waitForApp } from '../helpers/app-helpers';
import { hasAppChrome } from '../helpers/element-helpers';
import { isTauriDriver } from '../helpers/platform';

describe('Tauri app integration', () => {
  before(async () => {
    await waitForApp();
  });

  it('app chrome is visible (native integration)', async () => {
    expect(await hasAppChrome()).toBe(true);
  });

  it('app can take a screenshot (driver bridge works)', async () => {
    if (isTauriDriver()) {
      // tauri-driver does not support the W3C screenshot command —
      // verify the session is alive via getWindowHandle instead.
      const handle = await browser.getWindowHandle();
      expect(handle).toBeTruthy();
      console.log(
        '[TauriCommands] Screenshot not supported on tauri-driver; verified session handle'
      );
      return;
    }
    const screenshot = await browser.takeScreenshot();
    expect(screenshot).toBeTruthy();
    expect(screenshot.length).toBeGreaterThan(100);
  });
});
