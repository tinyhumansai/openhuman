import { elementExists, waitForApp } from '../helpers/app-helpers';

describe('Navigation', () => {
  before(async () => {
    await waitForApp();
  });

  it('app has menu items in the menu bar', async () => {
    // A running macOS app always has menu bar items
    const hasMenuItems = await elementExists('elementType == 56');
    // elementType 56 = XCUIElementTypeMenuBarItem
    expect(hasMenuItems).toBe(true);
  });
});
