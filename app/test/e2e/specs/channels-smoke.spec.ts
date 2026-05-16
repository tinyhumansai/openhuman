// @ts-nocheck
/**
 * Channels page smoke — Telegram + Discord panels render "not connected"
 * affordances (first slice of tinyhumansai/openhuman#290).
 *
 * Deferred to follow-up PRs:
 *   - Telegram / Discord OAuth happy path
 *   - Disconnect flow
 *   - Message send + inbound webhook
 *   - Auth edge cases / error states
 *
 * The page falls back to `FALLBACK_DEFINITIONS` (includes Telegram +
 * Discord) when core RPC has no live channel definitions — exactly the
 * "not_connected" state we assert here.
 */
import { waitForApp } from '../helpers/app-helpers';
import { textExists, waitForText } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-channels-smoke';

describe('Channels page smoke (Telegram + Discord)', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('renders Telegram and Discord channel panels in not-connected state', async () => {
    await navigateViaHash('/channels');

    await waitForText('Channels', 15_000);
    await waitForText('Telegram', 15_000);
    await waitForText('Discord', 15_000);

    expect(await textExists('Connect')).toBe(true);

    const clicked = await browser.execute(() => {
      const buttons = Array.from(document.querySelectorAll<HTMLButtonElement>('button'));
      const discordBtn = buttons.find(b => b.textContent?.includes('Discord'));
      if (discordBtn) {
        discordBtn.click();
        return true;
      }
      return false;
    });
    expect(clicked).toBe(true);

    await browser.pause(500);
    expect(await textExists('Connect')).toBe(true);
  });
});
