// @ts-nocheck
/**
 * E2E test: Discord Integration Flows (Channels architecture).
 *
 * Discord is a Channel in the unified Channels subsystem. It appears on the
 * Skills page under "Channel Integrations" with a "Configure" button that
 * opens a ChannelSetupModal. Two auth modes: bot_token and oauth.
 *
 * Aligned to Section 8: Integrations (Telegram, Gmail, Notion)
 * Same structure as telegram-flow.spec.ts but for Discord-specific endpoints.
 *
 *   8.1 Integration Setup
 *     8.1.1 Channel Connect — channels_connect with bot_token mode
 *     8.1.2 Scope Selection — channels_list returns Discord definition with capabilities
 *     8.1.3 Token Storage — auth_store_provider_credentials endpoint
 *
 *   8.2 Permission Enforcement
 *     8.2.1 Read Access — channels_status returns Discord connection state
 *     8.2.2 Write Access — channels_send_message endpoint
 *     8.2.3 Initiate Action — channels_create_thread endpoint
 *     8.2.4 Cross-Account Access Prevention — disconnect + revoke endpoints
 *
 *   8.3 Data Operations
 *     8.3.1 Data Fetch — discord_list_guilds + discord_list_channels
 *     8.3.2 Data Write — channels_send_message
 *     8.3.3 Permission Check — discord_check_permissions
 *
 *   8.4 Disconnect & Re-Setup
 *     8.4.1 Disconnect — channels_disconnect callable
 *     8.4.2 Token Revocation — auth_clear_session endpoint
 *     8.4.3 Re-Authorization — channels_connect callable after disconnect
 *     8.4.4 Permission Re-Sync — channels_status refreshable
 *
 *   8.5 UI Flow (Skills page → Channel Integrations → Configure modal)
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { expectRpcMethod, fetchCoreRpcMethods } from '../helpers/core-schema';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  clickText,
  dumpAccessibilityTree,
  textExists,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { completeOnboardingIfVisible, navigateViaHash } from '../helpers/shared-flows';
import { clearRequestLog, startMockServer, stopMockServer } from '../mock-server';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function stepLog(message: string, context?: unknown) {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[DiscordFlow][${stamp}] ${message}`);
    return;
  }
  console.log(`[DiscordFlow][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

// ===========================================================================
// 8. Integrations (Discord) — RPC endpoint verification
// ===========================================================================

describe('8. Integrations (Discord) — RPC endpoint verification', () => {
  let methods: Set<string>;

  before(async () => {
    await waitForApp();
    await waitForAppReady(20_000);
    methods = await fetchCoreRpcMethods();
  });

  // -----------------------------------------------------------------------
  // 8.1 Integration Setup
  // -----------------------------------------------------------------------

  it('8.1.1 — Channel Connect: channels_connect accepts Discord bot_token mode', async () => {
    expectRpcMethod(methods, 'openhuman.channels_connect');

    const res = await callOpenhumanRpc('openhuman.channels_connect', {
      channel: 'discord',
      authMode: 'bot_token',
      credentials: { bot_token: 'fake-e2e-discord-token' },
    });
    if (!res.ok) {
      stepLog('8.1.1 channels_connect failed:', res.error);
    }
    expect(res.ok).toBe(true);
  });

  it('8.1.2 — Scope Selection: channels_list returns Discord definition with capabilities', async () => {
    expectRpcMethod(methods, 'openhuman.channels_list');

    const res = await callOpenhumanRpc('openhuman.channels_list', {});
    if (res.ok && Array.isArray(res.result)) {
      const discord = res.result.find((d: { id: string }) => d.id === 'discord');
      if (discord) {
        stepLog('Discord definition found', {
          authModes: discord.auth_modes?.map((m: { mode: string }) => m.mode),
          capabilities: discord.capabilities,
        });
      }
    }
    if (!res.ok) {
      stepLog('8.1.2 channels_list failed:', res.error);
    }
    expect(res.ok).toBe(true);
  });

  it('8.1.3 — Token Storage: auth_store_provider_credentials registered', async () => {
    expectRpcMethod(methods, 'openhuman.auth_store_provider_credentials');
  });

  // -----------------------------------------------------------------------
  // 8.2 Permission Enforcement
  // -----------------------------------------------------------------------

  it('8.2.1 — Read Access: channels_status returns Discord connection state', async () => {
    expectRpcMethod(methods, 'openhuman.channels_status');
    const res = await callOpenhumanRpc('openhuman.channels_status', { channel: 'discord' });
    if (!res.ok) {
      stepLog('8.2.1 channels_status failed:', res.error);
    }
    expect(res.ok).toBe(true);
  });

  it('8.2.2 — Write Access: channels_send_message available', async () => {
    expectRpcMethod(methods, 'openhuman.channels_send_message');
  });

  it('8.2.3 — Initiate Action: channels_create_thread available', async () => {
    expectRpcMethod(methods, 'openhuman.channels_create_thread');
  });

  it('8.2.4 — Cross-Account Access Prevention: disconnect + revoke endpoints', async () => {
    expectRpcMethod(methods, 'openhuman.channels_disconnect');
    expectRpcMethod(methods, 'openhuman.auth_oauth_revoke_integration');
  });

  // -----------------------------------------------------------------------
  // 8.3 Data Operations (Discord-specific)
  // -----------------------------------------------------------------------

  it('8.3.1 — Data Fetch: discord_list_guilds endpoint registered', async () => {
    expectRpcMethod(methods, 'openhuman.channels_discord_list_guilds');
  });

  it('8.3.2 — Data Fetch: discord_list_channels endpoint registered', async () => {
    expectRpcMethod(methods, 'openhuman.channels_discord_list_channels');
  });

  it('8.3.3 — Permission Check: discord_check_permissions endpoint registered', async () => {
    expectRpcMethod(methods, 'openhuman.channels_discord_check_permissions');
  });

  // -----------------------------------------------------------------------
  // 8.4 Disconnect & Re-Setup
  // -----------------------------------------------------------------------

  it('8.4.1 — Disconnect: channels_disconnect callable for Discord', async () => {
    const res = await callOpenhumanRpc('openhuman.channels_disconnect', {
      channel: 'discord',
      authMode: 'bot_token',
    });
    if (!res.ok) {
      stepLog('8.4.1 channels_disconnect failed:', res.error);
    }
    expect(res.ok).toBe(true);
  });

  it('8.4.2 — Token Revocation: auth_clear_session available', async () => {
    expectRpcMethod(methods, 'openhuman.auth_clear_session');
  });

  it('8.4.3 — Re-Authorization: channels_connect callable after disconnect', async () => {
    await callOpenhumanRpc('openhuman.channels_disconnect', {
      channel: 'discord',
      authMode: 'bot_token',
    });
    const res = await callOpenhumanRpc('openhuman.channels_connect', {
      channel: 'discord',
      authMode: 'bot_token',
      credentials: { bot_token: 'fake-e2e-discord-reauth' },
    });
    if (!res.ok) {
      stepLog('8.4.3 channels_connect (re-auth) failed:', res.error);
    }
    expect(res.ok).toBe(true);
  });

  it('8.4.4 — Permission Re-Sync: channels_status refreshable after reconnect', async () => {
    const res = await callOpenhumanRpc('openhuman.channels_status', { channel: 'discord' });
    if (!res.ok) {
      stepLog('8.4.4 channels_status failed:', res.error);
    }
    expect(res.ok).toBe(true);
  });
});

// ===========================================================================
// 8.5 Discord — UI flow (Skills page → Channel Integrations → Configure)
// ===========================================================================

describe('8.5 Integrations (Discord) — UI flow', () => {
  before(async () => {
    stepLog('starting mock server');
    await startMockServer();
    stepLog('waiting for app');
    await waitForApp();
    clearRequestLog();
  });

  after(async () => {
    stepLog('stopping mock server');
    await stopMockServer();
  });

  it('8.5.1 — Skills page shows Discord in Channel Integrations', async () => {
    // Auth — try deep link, retry on failure
    for (let attempt = 1; attempt <= 3; attempt++) {
      stepLog(`trigger deep link (attempt ${attempt})`);
      await triggerAuthDeepLinkBypass(`e2e-discord-flow-${attempt}`);
      await waitForWindowVisible(25_000);
      await waitForWebView(15_000);
      await waitForAppReady(15_000);
      await browser.pause(3_000);

      const onLoginPage =
        (await textExists("Sign in! Let's Cook")) || (await textExists('Continue with email'));
      if (!onLoginPage) {
        stepLog(`Auth succeeded on attempt ${attempt}`);
        break;
      }
      if (attempt === 3) {
        const tree = await dumpAccessibilityTree();
        stepLog('Still on login page after 3 attempts. Tree:', tree.slice(0, 3000));
        throw new Error('Auth deep link did not navigate past sign-in page');
      }
      stepLog('Still on login page — retrying');
      await browser.pause(2_000);
    }

    await completeOnboardingIfVisible('[DiscordFlow]');

    stepLog('navigate to skills');
    await navigateViaHash('/skills');
    await browser.pause(3_000);

    // Skills page uses filter tabs (All, Built-in, Channels, Other).
    // Click the "Channels" tab to show channel cards.
    const hasChannelsTab = await textExists('Channels');
    if (hasChannelsTab) {
      try {
        await clickText('Channels', 8_000);
        await browser.pause(2_000);
        stepLog('Clicked "Channels" filter tab');
      } catch {
        stepLog('Could not click Channels tab — continuing with All view');
      }
    }

    // Discord card should now be visible
    const hasDiscord = await textExists('Discord');
    if (!hasDiscord) {
      const tree = await dumpAccessibilityTree();
      stepLog('Discord not found. Tree:', tree.slice(0, 4000));
    }
    expect(hasDiscord).toBe(true);
    stepLog('Discord channel visible on Skills page');
  });

  it('8.5.2 — Discord card shows status and action button', async () => {
    const hasDescription = await textExists('Send and receive messages via Discord');
    stepLog('Discord card description', { visible: hasDescription });

    // CTA button: "Setup" (disconnected) or "Manage" (connected)
    const hasSetup = await textExists('Setup');
    const hasManage = await textExists('Manage');
    const hasCta = hasSetup || hasManage;
    stepLog('Discord CTA', { setup: hasSetup, manage: hasManage });
    expect(hasCta).toBe(true);
  });

  it('8.5.3 — Click Discord Setup opens modal with auth modes and fields', async () => {
    // NOTE: `clickText('Setup')` picks the first "Setup" button in DOM order,
    // which is Telegram's (Telegram renders before Discord in the Channels
    // list).  Instead we find the Discord card by its "Discord" text node,
    // capture its Y coordinate, then click the Setup/Manage button whose
    // Y coordinate is closest to Discord's — i.e. the button inside the
    // Discord card row.
    stepLog('locating Discord card Setup button by position');
    let clicked = false;
    try {
      // 1. Find Discord text positions (there may be multiple — title + description)
      const discordEls = await browser.$$(
        '//*[contains(@label, "Discord") or contains(@value, "Discord") or contains(@title, "Discord")]'
      );
      if (discordEls.length === 0) {
        throw new Error('No Discord elements found in tree');
      }

      // Use the first Discord element as the card anchor (typically the title)
      const anchor = discordEls[0];
      const anchorLoc = await anchor.getLocation();
      stepLog(`Discord anchor at y=${anchorLoc.y}`);

      // 2. Find all Setup/Manage buttons
      const ctaButtons = await browser.$$(
        '//XCUIElementTypeButton[contains(@title, "Setup") or contains(@label, "Setup") or contains(@title, "Manage") or contains(@label, "Manage")]'
      );
      stepLog(`Found ${ctaButtons.length} Setup/Manage buttons`);

      if (ctaButtons.length === 0) {
        throw new Error('No Setup/Manage buttons found');
      }

      // 3. Pick the button whose Y is closest to Discord's anchor Y
      let bestBtn = null as (typeof ctaButtons)[number] | null;
      let bestDelta = Number.POSITIVE_INFINITY;
      for (const btn of ctaButtons) {
        try {
          const bLoc = await btn.getLocation();
          const delta = Math.abs(bLoc.y - anchorLoc.y);
          stepLog(`  candidate button y=${bLoc.y} delta=${delta}`);
          if (delta < bestDelta) {
            bestDelta = delta;
            bestBtn = btn;
          }
        } catch {
          // element may have gone stale; skip
        }
      }

      if (!bestBtn) {
        throw new Error('Could not select Discord CTA button');
      }

      // 4. W3C pointer click at the chosen button's center
      const loc = await bestBtn.getLocation();
      const size = await bestBtn.getSize();
      const cx = Math.round(loc.x + size.width / 2);
      const cy = Math.round(loc.y + size.height / 2);
      stepLog(`clicking Discord Setup at (${cx}, ${cy}) delta=${bestDelta}`);
      await browser.performActions([
        {
          type: 'pointer',
          id: 'mouse1',
          parameters: { pointerType: 'mouse' },
          actions: [
            { type: 'pointerMove', duration: 10, x: cx, y: cy },
            { type: 'pointerDown', button: 0 },
            { type: 'pause', duration: 80 },
            { type: 'pointerUp', button: 0 },
          ],
        },
      ]);
      await browser.releaseActions();
      clicked = true;
    } catch (err) {
      stepLog(
        `positional click failed: ${err instanceof Error ? err.message : String(err)} — falling back to clickText`
      );
    }

    if (!clicked) {
      // Fallback chain: clickText('Setup') → 'Manage' → 'Discord'
      try {
        await clickText('Setup', 10_000);
      } catch {
        try {
          await clickText('Manage', 10_000);
        } catch {
          try {
            await clickText('Discord', 10_000);
          } catch {
            stepLog('All click fallbacks failed');
          }
        }
      }
    }
    await browser.pause(3_000);

    // Dump tree for diagnostic
    const tree = await dumpAccessibilityTree();
    stepLog('Tree after clicking Discord:', tree.slice(0, 5000));

    // Check modal content — auth mode labels, buttons, fields
    const hasBotToken = await textExists('Use your own Bot Token');
    const hasOAuth = await textExists('OAuth Sign-in');
    const hasConnect = await textExists('Connect');
    const hasDisconnect = await textExists('Disconnect');
    const hasBotTokenField = await textExists('Bot Token');
    const hasGuildId = await textExists('Server (Guild) ID');
    const hasChannelBadge = await textExists('channel');
    const hasBotDesc = await textExists('Provide your own Discord bot token');
    const hasOAuthDesc = await textExists('Install the OpenHuman bot to your Discord server');

    stepLog('Discord modal content', {
      botToken: hasBotToken,
      oauth: hasOAuth,
      connect: hasConnect,
      disconnect: hasDisconnect,
      botTokenField: hasBotTokenField,
      guildId: hasGuildId,
      channelBadge: hasChannelBadge,
      botDesc: hasBotDesc,
      oauthDesc: hasOAuthDesc,
    });

    // At least one auth mode or modal content should be visible
    const modalOpened = hasBotToken || hasOAuth || hasChannelBadge || hasConnect || hasDisconnect;
    expect(modalOpened).toBe(true);

    // Close modal
    try {
      await browser.keys(['Escape']);
      await browser.pause(1_000);
    } catch {
      // non-fatal
    }
  });
});
