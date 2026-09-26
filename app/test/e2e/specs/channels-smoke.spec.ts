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
import { connectTelegramBot, disconnectTelegramBot } from '../helpers/telegram';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-channels-smoke';

describe('Channels page smoke (Telegram + Discord)', () => {
  before(async function beforeSuite() {
    this.timeout(90_000);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it.skip('renders Telegram and Discord channel panels in not-connected state', async function () {
    this.timeout(90_000);
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

  /**
   * 10.5.2 — the unified surface must distinguish a connected channel from a
   * disconnected one.
   *
   * The case above only ever sees the not-connected state, and the page falls
   * back to `FALLBACK_DEFINITIONS` when core RPC serves no live definitions
   * (`useChannelDefinitions.ts:83`, `:122`) — so it passes whether or not the
   * channel surface is wired to the core at all. That is what matrix row 10.5.2
   * means by "UI assertion shallow".
   *
   * This drives a real connect over RPC and asserts `ChannelStatusBadge`
   * (`channels.status.connected`) flips, then flips back on disconnect. The
   * connected/disconnected pair is the assertion: a badge stuck on one value
   * would satisfy either half alone.
   */
  it.skip('reflects a connected channel and returns to disconnected after disconnect (10.5.2)', async function () {
    this.timeout(120_000);

    // Known-disconnected baseline, so the "Connected" assertion below cannot
    // be satisfied by state left over from an earlier spec.
    await disconnectTelegramBot();
    await navigateViaHash('/channels');
    await waitForText('Telegram', 15_000);
    if (await textExists('Connected')) {
      throw new Error('precondition: Telegram should not already read Connected');
    }

    const connected = await connectTelegramBot({ botToken: '111111:e2e-channels-smoke-token' });
    expect(connected.ok).toBe(true);

    // Re-navigate rather than pause: the page reads definitions on mount.
    await navigateViaHash('/chat');
    await navigateViaHash('/channels');
    await waitForText('Telegram', 15_000);
    await waitForText('Connected', 20_000);

    await disconnectTelegramBot();
    await navigateViaHash('/chat');
    await navigateViaHash('/channels');
    await waitForText('Telegram', 15_000);
    await browser.waitUntil(async () => !(await textExists('Connected')), {
      timeout: 20_000,
      timeoutMsg:
        'the channel surface still reads Connected after channels_disconnect — the status ' +
        'badge is not reflecting the live channel state',
    });
  });
});
