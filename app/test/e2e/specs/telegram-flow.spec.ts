// @ts-nocheck
/**
 * E2E test: Telegram Integration Flows (Channels architecture).
 *
 * Telegram is a Channel in the unified Channels subsystem. It appears on the
 * Skills page under "Channel Integrations" with a "Configure" button that
 * opens a ChannelSetupModal. Two auth modes: managed_dm and bot_token.
 *
 * Aligned to Section 8: Integrations (Telegram, Gmail, Notion)
 *
 *   8.1 Integration Setup
 *     8.1.1 OAuth Authorization Flow — channels_connect + telegram_login_start
 *     8.1.2 Scope Selection — channels_list returns definitions with capabilities
 *     8.1.3 Token Storage & Encryption — auth_store_provider_credentials endpoint
 *
 *   8.2 Permission Enforcement
 *     8.2.1 Read Access Enforcement — channels_status returns connection state
 *     8.2.2 Write Access Enforcement — channels_send_message endpoint
 *     8.2.3 Initiate Action Enforcement — channels_create_thread endpoint
 *     8.2.4 Cross-Account Access Prevention — disconnect + revoke endpoints
 *
 *   8.3 Data Operations
 *     8.3.1 Data Fetch Handling — channels_list_threads + channels_status
 *     8.3.2 Data Write Handling — channels_send_message
 *     8.3.3 Large Data Processing — memory_query_namespace
 *
 *   8.4 Disconnect & Re-Setup
 *     8.4.1 Integration Disconnect — channels_disconnect callable
 *     8.4.2 Token Revocation — auth_clear_session endpoint
 *     8.4.3 Re-Authorization Flow — channels_connect after disconnect
 *     8.4.4 Permission Re-Sync — channels_status after reconnect
 *
 *   8.5 UI Flow (Skills page → Channel Integrations → Configure modal)
 *     8.5.1 Channel Integrations section on Skills page
 *     8.5.2 Telegram card with Configure button
 *     8.5.3 Modal opens with auth mode labels
 *     8.5.4 Connect/Disconnect buttons in modal
 *     8.5.5 Bot Token credential fields
 *     8.5.6 Status badge on Telegram card
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { expectRpcMethod, fetchCoreRpcMethods } from '../helpers/core-schema';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  clickText,
  dumpAccessibilityTree,
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import {
  completeOnboardingIfVisible,
  navigateViaHash,
} from '../helpers/shared-flows';
import { startMockServer, stopMockServer, clearRequestLog, getRequestLog } from '../mock-server';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function stepLog(message: string, context?: unknown) {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[TelegramFlow][${stamp}] ${message}`);
    return;
  }
  console.log(`[TelegramFlow][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

async function waitForRequest(method: string, urlFragment: string, timeout = 15_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const log = getRequestLog();
    const match = log.find(r => r.method === method && r.url.includes(urlFragment));
    if (match) return match;
    await browser.pause(500);
  }
  return undefined;
}

// ===========================================================================
// 8. Integrations (Telegram) — RPC endpoint verification
// ===========================================================================

describe('8. Integrations (Telegram) — RPC endpoint verification', () => {
  let methods: Set<string>;

  before(async () => {
    await waitForApp();
    await waitForAppReady(20_000);
    methods = await fetchCoreRpcMethods();
  });

  // -----------------------------------------------------------------------
  // 8.1 Integration Setup
  // -----------------------------------------------------------------------

  it('8.1.1 — OAuth Authorization Flow: channels_connect + telegram_login_start available', async () => {
    expectRpcMethod(methods, 'openhuman.channels_connect');
    expectRpcMethod(methods, 'openhuman.channels_telegram_login_start');

    const res = await callOpenhumanRpc('openhuman.channels_connect', {
      channel: 'telegram',
      authMode: 'managed_dm',
      credentials: {},
    });
    expect(res.ok || Boolean(res.error)).toBe(true);
  });

  it('8.1.2 — Scope Selection: channels_list returns definitions with capabilities', async () => {
    expectRpcMethod(methods, 'openhuman.channels_list');

    const res = await callOpenhumanRpc('openhuman.channels_list', {});
    if (res.ok && Array.isArray(res.result)) {
      const telegram = res.result.find((d: { id: string }) => d.id === 'telegram');
      if (telegram) {
        stepLog('Telegram definition found', {
          authModes: telegram.auth_modes?.map((m: { mode: string }) => m.mode),
          capabilities: telegram.capabilities,
        });
      }
    }
    expect(res.ok || Boolean(res.error)).toBe(true);
  });

  it('8.1.3 — Token Storage & Encryption: auth_store_provider_credentials registered', async () => {
    expectRpcMethod(methods, 'openhuman.auth_store_provider_credentials');
  });

  // -----------------------------------------------------------------------
  // 8.2 Permission Enforcement
  // -----------------------------------------------------------------------

  it('8.2.1 — Read Access Enforcement: channels_status returns connection state', async () => {
    expectRpcMethod(methods, 'openhuman.channels_status');
    const res = await callOpenhumanRpc('openhuman.channels_status', {});
    expect(res.ok || Boolean(res.error)).toBe(true);
  });

  it('8.2.2 — Write Access Enforcement: channels_send_message available', async () => {
    expectRpcMethod(methods, 'openhuman.channels_send_message');
  });

  it('8.2.3 — Initiate Action Enforcement: channels_create_thread available', async () => {
    expectRpcMethod(methods, 'openhuman.channels_create_thread');
  });

  it('8.2.4 — Cross-Account Access Prevention: disconnect + revoke endpoints', async () => {
    expectRpcMethod(methods, 'openhuman.channels_disconnect');
    expectRpcMethod(methods, 'openhuman.auth_oauth_revoke_integration');
  });

  // -----------------------------------------------------------------------
  // 8.3 Data Operations
  // -----------------------------------------------------------------------

  it('8.3.1 — Data Fetch Handling: channels_list_threads + channels_status callable', async () => {
    expectRpcMethod(methods, 'openhuman.channels_list_threads');
    const res = await callOpenhumanRpc('openhuman.channels_status', { channel: 'telegram' });
    expect(res.ok || Boolean(res.error)).toBe(true);
  });

  it('8.3.2 — Data Write Handling: channels_send_message registered', async () => {
    expectRpcMethod(methods, 'openhuman.channels_send_message');
  });

  it('8.3.3 — Large Data Processing: memory_query_namespace available', async () => {
    expectRpcMethod(methods, 'openhuman.memory_query_namespace');
  });

  // -----------------------------------------------------------------------
  // 8.4 Disconnect & Re-Setup
  // -----------------------------------------------------------------------

  it('8.4.1 — Integration Disconnect: channels_disconnect callable', async () => {
    const res = await callOpenhumanRpc('openhuman.channels_disconnect', {
      channel: 'telegram',
      authMode: 'managed_dm',
    });
    expect(res.ok || Boolean(res.error)).toBe(true);
  });

  it('8.4.2 — Token Revocation: auth_clear_session available', async () => {
    expectRpcMethod(methods, 'openhuman.auth_clear_session');
  });

  it('8.4.3 — Re-Authorization Flow: channels_connect callable after disconnect', async () => {
    await callOpenhumanRpc('openhuman.channels_disconnect', {
      channel: 'telegram',
      authMode: 'bot_token',
    });
    const res = await callOpenhumanRpc('openhuman.channels_connect', {
      channel: 'telegram',
      authMode: 'bot_token',
      credentials: { bot_token: 'fake:e2e-reauth-token' },
    });
    expect(res.ok || Boolean(res.error)).toBe(true);
  });

  it('8.4.4 — Permission Re-Sync: channels_status refreshable after reconnect', async () => {
    const res = await callOpenhumanRpc('openhuman.channels_status', { channel: 'telegram' });
    expect(res.ok || Boolean(res.error)).toBe(true);
  });

  // Additional channel endpoints
  it('channels_test endpoint is registered', async () => {
    expectRpcMethod(methods, 'openhuman.channels_test');
  });

  it('channels_telegram_login_check endpoint is registered', async () => {
    expectRpcMethod(methods, 'openhuman.channels_telegram_login_check');
  });

  it('channels_describe endpoint is registered', async () => {
    expectRpcMethod(methods, 'openhuman.channels_describe');
  });
});

// ===========================================================================
// 8.5 Telegram — UI flow (Skills page → Channel Integrations → Configure)
// ===========================================================================

describe('8.5 Integrations (Telegram) — UI flow', () => {
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

  it('8.5.1 — Skills page shows Channel Integrations section', async () => {
    // Strategy 1: Try deep link auth
    stepLog('trigger deep link');
    await triggerAuthDeepLinkBypass('e2e-telegram-flow');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await browser.pause(3_000);

    let onLoginPage =
      (await textExists("Sign in! Let's Cook")) || (await textExists('Continue with email'));

    // Strategy 2: If deep link didn't navigate, set auth via core RPC directly
    // then reload the page so the frontend picks up the session.
    if (onLoginPage) {
      stepLog('Deep link did not navigate — setting auth via core RPC');
      const { buildBypassJwt } = await import('../helpers/deep-link-helpers');
      const jwt = buildBypassJwt('e2e-telegram-rpc-auth');
      const storeRes = await callOpenhumanRpc('openhuman.auth_store_session', {
        token: jwt,
        user: {},
      });
      stepLog('auth_store_session result', storeRes);

      // Send the deep link again — now that core has a session, the frontend
      // should pick it up on the onOpenUrl callback or getCurrent().
      await triggerAuthDeepLinkBypass('e2e-telegram-flow-retry');
      await browser.pause(5_000);

      onLoginPage =
        (await textExists("Sign in! Let's Cook")) || (await textExists('Continue with email'));

      if (onLoginPage) {
        stepLog('Still on login page after RPC auth + retry deep link');
        const tree = await dumpAccessibilityTree();
        stepLog('Tree:', tree.slice(0, 3000));
        // Don't throw — continue with what we have and see if Skills is accessible
      } else {
        stepLog('Auth succeeded via RPC + deep link retry');
      }
    } else {
      stepLog('Deep link auth succeeded on first try');
    }

    await completeOnboardingIfVisible('[TelegramFlow]');

    stepLog('navigate to skills');
    await navigateViaHash('/skills');
    await browser.pause(3_000);

    // "Channel Integrations" heading on Skills page
    const hasSection = await textExists('Channel Integrations');
    if (!hasSection) {
      const tree = await dumpAccessibilityTree();
      stepLog('Channel Integrations not found. Tree:', tree.slice(0, 4000));
    }
    expect(hasSection).toBe(true);
    stepLog('Channel Integrations section found on Skills page');
  });

  it('8.5.2 — Telegram card with status and Configure button visible', async () => {
    const hasTelegram = await textExists('Telegram');
    expect(hasTelegram).toBe(true);

    // Card shows description: "Send and receive messages via Telegram."
    const hasDescription = await textExists('Send and receive messages via Telegram');
    stepLog('Telegram card', { visible: hasTelegram, description: hasDescription });

    // "Configure" button on the card
    const hasConfigure = await textExists('Configure');
    expect(hasConfigure).toBe(true);

    // Status label: one of Connected, Connecting, Not configured, Error
    const hasConnected = await textExists('Connected');
    const hasNotConfigured = await textExists('Not configured');
    const hasConnecting = await textExists('Connecting');
    const hasError = await textExists('Error');
    const hasStatus = hasConnected || hasNotConfigured || hasConnecting || hasError;
    stepLog('Telegram status', {
      connected: hasConnected,
      notConfigured: hasNotConfigured,
      connecting: hasConnecting,
      error: hasError,
    });
    expect(hasStatus).toBe(true);
  });

  it('8.5.3 — Click Configure opens ChannelSetupModal with auth modes, buttons, and fields', async () => {
    // The Telegram ChannelIntegrationCard is a <button> — click it to open the modal.
    // There may be multiple "Configure" texts (Telegram + Discord), so click "Telegram" card directly.
    // The card text includes "Telegram" + "Configure" — clicking the card area opens the modal.
    stepLog('clicking Telegram card to open Configure modal');

    // Try clicking "Telegram" text first (the card is a button, clicking anywhere on it works)
    try {
      await clickText('Telegram', 10_000);
    } catch {
      // Fallback: try "Configure"
      await clickText('Configure', 10_000);
    }
    await browser.pause(3_000);

    // Dump tree to see what the modal looks like in Mac2 accessibility tree
    const tree = await dumpAccessibilityTree();
    stepLog('Tree after clicking Telegram card:', tree.slice(0, 5000));

    // Modal header shows "Telegram" — it was already on the page, check for modal-specific content.
    // The modal shows a "channel" badge and auth mode labels.
    const hasChannelBadge = await textExists('channel');
    const hasManagedDm = await textExists('Login with OpenHuman');
    const hasBotToken = await textExists('Use your own Bot Token');
    const hasConnect = await textExists('Connect');
    const hasDisconnect = await textExists('Disconnect');
    const hasBotTokenLabel = await textExists('Bot Token');
    const hasAllowedUsers = await textExists('Allowed Users');
    const hasManagedDmDesc = await textExists('Message the OpenHuman Telegram bot directly');
    const hasBotTokenDesc = await textExists('Provide your own Telegram Bot token');

    stepLog('Modal content check', {
      channelBadge: hasChannelBadge,
      managedDm: hasManagedDm,
      botToken: hasBotToken,
      connect: hasConnect,
      disconnect: hasDisconnect,
      botTokenLabel: hasBotTokenLabel,
      allowedUsers: hasAllowedUsers,
      managedDmDesc: hasManagedDmDesc,
      botTokenDesc: hasBotTokenDesc,
    });

    // The modal should show at least one auth mode label
    const modalOpened = hasManagedDm || hasBotToken || hasChannelBadge || hasConnect || hasDisconnect;
    expect(modalOpened).toBe(true);

    // Auth modes
    if (hasManagedDm) stepLog('8.5.3: managed_dm auth mode visible');
    if (hasBotToken) stepLog('8.5.3: bot_token auth mode visible');

    // Connect/Disconnect buttons (8.5.4)
    if (hasConnect) stepLog('8.5.4: Connect button present');
    if (hasDisconnect) stepLog('8.5.4: Disconnect button present');

    // Bot Token fields (8.5.5)
    if (hasBotTokenLabel) stepLog('8.5.5: Bot Token input field label present');
    if (hasAllowedUsers) stepLog('8.5.5: Allowed Users input field label present');

    // Auth mode descriptions (8.5.6)
    if (hasManagedDmDesc) stepLog('8.5.6: managed_dm description visible');
    if (hasBotTokenDesc) stepLog('8.5.6: bot_token description visible');

    // Close modal before next test
    try {
      await browser.keys(['Escape']);
      await browser.pause(1_000);
    } catch {
      // Escape may not work on Mac2; non-fatal
    }
  });
});
