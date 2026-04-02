import { waitForApp } from '../helpers/app-helpers';
import { hasAppChrome } from '../helpers/element-helpers';

describe('Navigation', () => {
  before(async () => {
    await waitForApp();
  });

  it('app has visible chrome (menu bar on macOS, window handle on Linux)', async () => {
    expect(await hasAppChrome()).toBe(true);
  });
});
