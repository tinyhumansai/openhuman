import { waitForApp } from '../helpers/app-helpers';
import { hasAppChrome } from '../helpers/element-helpers';

describe('Smoke tests', () => {
  before(async () => {
    await waitForApp();
  });

  it('app process launched successfully (session created)', async () => {
    // Verify the driver has an active session connected to the app
    const sessionId = browser.sessionId;
    expect(sessionId).toBeDefined();
    expect(typeof sessionId).toBe('string');
    expect(sessionId.length).toBeGreaterThan(0);
  });

  it('app chrome is visible (menu bar on macOS, window on Linux)', async () => {
    expect(await hasAppChrome()).toBe(true);
  });

  it('app page source has elements', async () => {
    // Find any element in the app to confirm the driver can see it
    const elements = await browser.$$('//*');
    expect(elements.length).toBeGreaterThan(0);
  });
});
