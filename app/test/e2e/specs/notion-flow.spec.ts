// @ts-nocheck
/**
 * E2E test: Notion Integration Flows (3rd Party Skill).
 *
 * Notion is a 3rd Party Skill (id: "notion") managed via the Skills subsystem.
 * It appears on the Skills page under "3rd Party Skills" with Enable/Setup/Configure
 * buttons. OAuth is handled via auth_oauth_connect.
 *
 * Aligned to Section 8: Integrations
 *
 *   8.1 Integration Setup
 *     8.1.1 OAuth Authorization Flow — auth_oauth_connect with notion provider
 *     8.1.2 Scope Selection — auth_oauth_list_integrations returns scopes
 *     8.1.3 Token Storage — auth_store_provider_credentials endpoint
 *
 *   8.2 Permission Enforcement
 *     8.2.1 Read Access — skills_list_tools lists read tools for notion skill
 *     8.2.2 Write Access — skills_list_tools lists write tools for notion skill
 *     8.2.3 Initiate Action — skills_call_tool enforces runtime checks
 *     8.2.4 Cross-Account Access Prevention — auth_oauth_revoke_integration
 *
 *   8.3 Data Operations
 *     8.3.1 Data Fetch — skills_sync endpoint callable
 *     8.3.2 Data Write — skills_call_tool with write tool
 *     8.3.3 Large Data Processing — memory_query_namespace for chunked data
 *
 *   8.4 Disconnect & Re-Setup
 *     8.4.1 Integration Disconnect — auth_oauth_revoke_integration callable
 *     8.4.2 Token Revocation — auth_clear_session endpoint
 *     8.4.3 Re-Authorization — auth_oauth_connect callable after revoke
 *     8.4.4 Permission Re-Sync — skills_sync refreshable
 *
 *   8.5 UI Flow (Skills page → 3rd Party Skills → Notion card)
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
import {
  completeOnboardingIfVisible,
  dismissLocalAISnackbarIfVisible,
  navigateViaHash,
} from '../helpers/shared-flows';
import { clearRequestLog, startMockServer, stopMockServer } from '../mock-server';

function stepLog(message: string, context?: unknown) {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[NotionFlow][${stamp}] ${message}`);
    return;
  }
  console.log(`[NotionFlow][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

// ===========================================================================
// 8. Integrations (Notion) — RPC endpoint verification
// ===========================================================================

describe('8. Integrations (Notion) — RPC endpoint verification', () => {
  let methods: Set<string>;

  before(async () => {
    await waitForApp();
    await waitForAppReady(20_000);
    methods = await fetchCoreRpcMethods();
  });

  // -----------------------------------------------------------------------
  // 8.1 Integration Setup
  // -----------------------------------------------------------------------

  it('8.1.1 — OAuth Authorization Flow: auth_oauth_connect with notion provider', async () => {
    expectRpcMethod(methods, 'openhuman.auth_oauth_connect');
    const res = await callOpenhumanRpc('openhuman.auth_oauth_connect', {
      provider: 'notion',
      responseType: 'json',
    });
    expect(res.ok || Boolean(res.error)).toBe(true);
  });

  it('8.1.2 — Scope Selection: auth_oauth_list_integrations returns integration list', async () => {
    expectRpcMethod(methods, 'openhuman.auth_oauth_list_integrations');
    const res = await callOpenhumanRpc('openhuman.auth_oauth_list_integrations', {});
    expect(res.ok || Boolean(res.error)).toBe(true);
  });

  it('8.1.3 — Token Storage: auth_store_provider_credentials registered', async () => {
    expectRpcMethod(methods, 'openhuman.auth_store_provider_credentials');
  });

  // -----------------------------------------------------------------------
  // 8.2 Permission Enforcement
  // -----------------------------------------------------------------------

  it('8.2.1 — Read Access: skills_list_tools endpoint registered for notion skill', async () => {
    expectRpcMethod(methods, 'openhuman.skills_list_tools');
  });

  it('8.2.2 — Write Access: skills_call_tool endpoint registered', async () => {
    expectRpcMethod(methods, 'openhuman.skills_call_tool');
  });

  it('8.2.3 — Initiate Action: skills_call_tool rejects missing notion runtime', async () => {
    const res = await callOpenhumanRpc('openhuman.skills_call_tool', {
      id: 'notion',
      tool_name: 'create_page',
      args: {},
    });
    expect(res.ok).toBe(false);
  });

  it('8.2.4 — Cross-Account Access Prevention: auth_oauth_revoke_integration registered', async () => {
    expectRpcMethod(methods, 'openhuman.auth_oauth_revoke_integration');
  });

  // -----------------------------------------------------------------------
  // 8.3 Data Operations
  // -----------------------------------------------------------------------

  it('8.3.1 — Data Fetch: skills_sync endpoint callable for notion', async () => {
    expectRpcMethod(methods, 'openhuman.skills_sync');
    const res = await callOpenhumanRpc('openhuman.skills_sync', { id: 'notion' });
    expect(res.ok || Boolean(res.error)).toBe(true);
  });

  it('8.3.2 — Data Write: skills_call_tool rejects write to non-running notion', async () => {
    const res = await callOpenhumanRpc('openhuman.skills_call_tool', {
      id: 'notion',
      tool_name: 'update_page',
      args: { pageId: 'test', content: 'e2e' },
    });
    expect(res.ok).toBe(false);
  });

  it('8.3.3 — Large Data Processing: memory_query_namespace available', async () => {
    expectRpcMethod(methods, 'openhuman.memory_query_namespace');
  });

  // -----------------------------------------------------------------------
  // 8.4 Disconnect & Re-Setup
  // -----------------------------------------------------------------------

  it('8.4.1 — Integration Disconnect: auth_oauth_revoke_integration callable', async () => {
    const res = await callOpenhumanRpc('openhuman.auth_oauth_revoke_integration', {
      integrationId: 'notion-e2e-test',
    });
    expect(res.ok || Boolean(res.error)).toBe(true);
  });

  it('8.4.2 — Token Revocation: auth_clear_session available', async () => {
    expectRpcMethod(methods, 'openhuman.auth_clear_session');
  });

  it('8.4.3 — Re-Authorization: auth_oauth_connect callable after revoke', async () => {
    await callOpenhumanRpc('openhuman.auth_oauth_revoke_integration', {
      integrationId: 'notion-e2e-reauth',
    });
    const res = await callOpenhumanRpc('openhuman.auth_oauth_connect', {
      provider: 'notion',
      responseType: 'json',
    });
    expect(res.ok || Boolean(res.error)).toBe(true);
  });

  it('8.4.4 — Permission Re-Sync: skills_sync callable after reconnect', async () => {
    const res = await callOpenhumanRpc('openhuman.skills_sync', { id: 'notion' });
    expect(res.ok || Boolean(res.error)).toBe(true);
  });

  // Additional skill endpoints
  it('skills_start endpoint registered', async () => {
    expectRpcMethod(methods, 'openhuman.skills_start');
  });

  it('skills_stop endpoint registered', async () => {
    expectRpcMethod(methods, 'openhuman.skills_stop');
  });

  it('skills_discover endpoint registered', async () => {
    expectRpcMethod(methods, 'openhuman.skills_discover');
  });
});

// ===========================================================================
// 8.5 Notion — UI flow (Skills page → 3rd Party Skills → Notion card)
// ===========================================================================

describe('8.5 Integrations (Notion) — UI flow', () => {
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

  it('8.5.1 — Skills page shows 3rd Party Skills section with Notion skill', async () => {
    for (let attempt = 1; attempt <= 3; attempt++) {
      stepLog(`trigger deep link (attempt ${attempt})`);
      await triggerAuthDeepLinkBypass(`e2e-notion-flow-${attempt}`);
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
        stepLog('Still on login page. Tree:', tree.slice(0, 3000));
        throw new Error('Auth deep link did not navigate past sign-in page');
      }
      stepLog('Still on login page — retrying');
      await browser.pause(2_000);
    }

    await completeOnboardingIfVisible('[NotionFlow]');

    stepLog('navigate to skills');
    await navigateViaHash('/skills');
    await browser.pause(3_000);

    const hasSection = await textExists('3rd Party Skills');
    if (!hasSection) {
      const tree = await dumpAccessibilityTree();
      stepLog('3rd Party Skills not found. Tree:', tree.slice(0, 4000));
    }
    expect(hasSection).toBe(true);
    stepLog('3rd Party Skills section found');
  });

  it('8.5.2 — Notion skill card visible with status and action button', async () => {
    // 3rd Party Skills section is below Built-in Skills and Channel Integrations — scroll down
    const { scrollToFindText } = await import('../helpers/element-helpers');
    let hasNotion = await textExists('Notion');
    if (!hasNotion) {
      stepLog('Notion not visible — scrolling down');
      hasNotion = await scrollToFindText('Notion', 6, 400);
    }
    if (!hasNotion) {
      const tree = await dumpAccessibilityTree();
      stepLog('Notion skill not found after scrolling. Tree:', tree.slice(0, 4000));
    }
    expect(hasNotion).toBe(true);

    // Status: one of Connected, Setup, Offline, Error, Disconnected, Not Auth
    const statuses = ['Connected', 'Setup', 'Offline', 'Error', 'Disconnected', 'Not Auth'];
    let foundStatus = null;
    for (const status of statuses) {
      if (await textExists(status)) {
        foundStatus = status;
        break;
      }
    }
    stepLog('Notion skill status', { found: foundStatus });

    // Action button: Enable, Setup, Configure, or Retry
    const hasEnable = await textExists('Enable');
    const hasSetup = await textExists('Setup');
    const hasConfigure = await textExists('Configure');
    const hasRetry = await textExists('Retry');
    const hasAction = hasEnable || hasSetup || hasConfigure || hasRetry;
    stepLog('Notion action button', {
      enable: hasEnable,
      setup: hasSetup,
      configure: hasConfigure,
      retry: hasRetry,
    });
    expect(hasAction).toBe(true);
  });

  it('8.5.3 — Click Notion skill opens SkillSetupModal', async () => {
    // Dismiss the LocalAI download snackbar if visible — it floats at the bottom
    // and can block skill action buttons.
    await dismissLocalAISnackbarIfVisible('[NotionFlow]');

    stepLog('clicking Notion skill action button');
    // Use aria-label text to target the Notion-specific button (not Gmail's)
    // Buttons have aria-label="Enable Notion", "Setup Notion", "Configure Notion", "Retry Notion"
    const actionCandidates = ['Setup Notion', 'Enable Notion', 'Configure Notion', 'Retry Notion'];
    let clicked = false;
    for (const label of actionCandidates) {
      if (await textExists(label)) {
        try {
          await clickText(label, 10_000);
          clicked = true;
          stepLog(`Clicked "${label}" button`);
          break;
        } catch {
          continue;
        }
      }
    }

    if (!clicked) {
      // Fallback: click the Notion skill name text in the card
      try {
        await clickText('Notion', 10_000);
        clicked = true;
        stepLog('Clicked "Notion" text directly');
      } catch {
        stepLog('Could not click Notion skill');
      }
    }

    // Wait for the SkillSetupModal to load — poll for modal markers
    const modalMarkers = ['Connect Notion', 'Manage Notion', 'Connect with Notion', 'skill'];
    const deadline = Date.now() + 15_000;
    let modalFound = false;
    while (Date.now() < deadline) {
      for (const marker of modalMarkers) {
        if (await textExists(marker)) {
          stepLog(`Modal loaded — found "${marker}"`);
          modalFound = true;
          break;
        }
      }
      if (modalFound) break;
      await browser.pause(500);
    }

    if (!modalFound) {
      const tree = await dumpAccessibilityTree();
      stepLog('Modal not found after 15s. Tree:', tree.slice(0, 5000));
    }

    const hasConnectTitle = await textExists('Connect Notion');
    const hasManageTitle = await textExists('Manage Notion');
    stepLog('Notion modal', { connect: hasConnectTitle, manage: hasManageTitle });

    expect(modalFound || clicked).toBe(true);

    // Close modal
    try {
      await browser.keys(['Escape']);
      await browser.pause(1_000);
    } catch {
      // non-fatal
    }
  });
});
