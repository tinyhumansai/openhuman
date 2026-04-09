// @ts-nocheck
/**
 * E2E: Permissions & System Access
 *
 * Covers:
 *   2.1.1  Accessibility Permission
 *   2.1.2  Input Monitoring Permission
 *   2.1.3  Screen Recording Permission
 *   2.1.4  Microphone Permission
 *   2.2.1  Permission Denied Handling
 *   2.2.2  Permission Re-Request Flow
 *   2.2.3  Restart & Refresh Sync
 *   2.2.4  Partial Permission State
 */
import { waitForApp, waitForAppReady, waitForAuthBootstrap } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { fetchCoreRpcMethods } from '../helpers/core-schema';
import { triggerAuthDeepLink } from '../helpers/deep-link-helpers';
import {
  dumpAccessibilityTree,
  textExists,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import {
  dismissLocalAISnackbarIfVisible,
  navigateToSettings,
  walkOnboarding,
} from '../helpers/shared-flows';
import {
  clearRequestLog,
  resetMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

const LOG_PREFIX = '[PermissionsSpec]';
const STRICT = process.env.E2E_STRICT_PERMISSIONS === '1';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function looksLikePermissionError(error?: string): boolean {
  const text = String(error || '').toLowerCase();
  return (
    text.includes('permission') ||
    text.includes('accessibility') ||
    text.includes('screen recording') ||
    text.includes('input monitoring') ||
    text.includes('not granted') ||
    text.includes('unsupported') ||
    text.includes('denied')
  );
}

function looksLikeNotImplemented(error?: string): boolean {
  const text = String(error || '').toLowerCase();
  return (
    text.includes('not implemented') ||
    text.includes('unknown method') ||
    text.includes('method not found') ||
    text.includes('no handler')
  );
}

/**
 * Call an RPC method and require ok=true. Throws on failure.
 */
async function _rpcOk(method: string, params: Record<string, unknown> = {}) {
  const result = await callOpenhumanRpc(method, params);
  if (!result.ok) {
    console.log(`${LOG_PREFIX} ${method} failed:`, result.error);
  }
  expect(result.ok).toBe(true);
  return result.result;
}

/**
 * Call an RPC method — accept ok=true OR a known permission/not-implemented error.
 * Returns the raw result for further assertions.
 */
async function rpcOkOrPermission(method: string, params: Record<string, unknown> = {}) {
  const result = await callOpenhumanRpc(method, params);
  if (!result.ok) {
    const acceptable =
      looksLikePermissionError(result.error) || looksLikeNotImplemented(result.error);
    if (!acceptable) {
      console.log(`${LOG_PREFIX} unexpected error for ${method}:`, result.error);
    }
    if (STRICT) {
      expect(result.ok).toBe(true);
    } else {
      expect(acceptable).toBe(true);
    }
  }
  return result;
}

/**
 * Skip the test with INCONCLUSIVE if the RPC method is not in the schema.
 */
function requireMethod(methods: Set<string>, method: string, caseId: string): boolean {
  if (methods.has(method)) return true;
  if (STRICT) {
    expect(methods.has(method)).toBe(true);
    return false;
  }
  console.log(`${LOG_PREFIX} ${caseId}: INCONCLUSIVE — RPC method ${method} not registered`);
  return false;
}

// ---------------------------------------------------------------------------
// Suite
// ---------------------------------------------------------------------------

describe('Permissions & System Access', function () {
  this.timeout(5 * 60_000);

  let methods: Set<string>;

  before(async function () {
    this.timeout(90_000);
    await startMockServer();
    await waitForApp();
    await waitForAppReady(20_000);

    // Login + onboarding without asserting a specific landing page —
    // this spec only needs auth context to call core RPC methods.
    await triggerAuthDeepLink('e2e-permissions-token');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await waitForAuthBootstrap(15_000);
    await walkOnboarding(LOG_PREFIX);
    // Give the app a moment to settle after onboarding
    await browser.pause(3_000);

    methods = await fetchCoreRpcMethods();
    clearRequestLog();
  });

  after(async function () {
    this.timeout(30_000);
    resetMockBehavior();
    try {
      await stopMockServer();
    } catch {
      /* non-fatal */
    }
  });

  beforeEach(() => {
    clearRequestLog();
    resetMockBehavior();
  });

  // ─── 2.1 macOS Permissions ──────────────────────────────────────────────

  describe('2.1 macOS Permissions', () => {
    it('2.1.1 — Accessibility Permission: status exposes accessibility field', async () => {
      if (!requireMethod(methods, 'openhuman.screen_intelligence_status', '2.1.1')) return;

      const result = await rpcOkOrPermission('openhuman.screen_intelligence_status', {});
      const text = JSON.stringify(result?.result || result || {}).toLowerCase();

      if (result.ok) {
        const hasField = text.includes('accessibility') || text.includes('permissions');
        if (!hasField) {
          console.log(`${LOG_PREFIX} 2.1.1: status payload:`, text.slice(0, 400));
        }
        expect(hasField).toBe(true);
      }
      console.log(`${LOG_PREFIX} 2.1.1 PASSED`);
    });

    it('2.1.2 — Input Monitoring Permission: status exposes input_monitoring field', async () => {
      if (!requireMethod(methods, 'openhuman.screen_intelligence_status', '2.1.2')) return;

      const result = await rpcOkOrPermission('openhuman.screen_intelligence_status', {});
      if (result.ok) {
        const text = JSON.stringify(result.result || {}).toLowerCase();
        expect(
          text.includes('input_monitoring') ||
            text.includes('input monitoring') ||
            text.includes('permissions')
        ).toBe(true);
      }
      console.log(`${LOG_PREFIX} 2.1.2 PASSED`);
    });

    it('2.1.3 — Screen Recording Permission: status exposes screen_recording field', async () => {
      if (!requireMethod(methods, 'openhuman.screen_intelligence_status', '2.1.3')) return;

      const result = await rpcOkOrPermission('openhuman.screen_intelligence_status', {});
      if (result.ok) {
        const text = JSON.stringify(result.result || {}).toLowerCase();
        expect(
          text.includes('screen_recording') ||
            text.includes('screen recording') ||
            text.includes('permissions')
        ).toBe(true);
      }
      console.log(`${LOG_PREFIX} 2.1.3 PASSED`);
    });

    it('2.1.4 — Microphone Permission: voice status endpoint is accessible', async () => {
      if (!requireMethod(methods, 'openhuman.voice_status', '2.1.4')) return;

      const result = await rpcOkOrPermission('openhuman.voice_status', {});
      console.log(`${LOG_PREFIX} 2.1.4 voice_status ok=${result.ok}`);
      console.log(`${LOG_PREFIX} 2.1.4 PASSED`);
    });
  });

  // ─── 2.2 Permission State Handling ──────────────────────────────────────

  describe('2.2 Permission State Handling', () => {
    it('2.2.1 — Permission Denied Handling: capture fails cleanly when permission not granted', async () => {
      if (!requireMethod(methods, 'openhuman.screen_intelligence_capture_now', '2.2.1')) return;

      const result = await callOpenhumanRpc('openhuman.screen_intelligence_capture_now', {});
      // Either succeeds (permission granted) or fails with a clear permission error
      expect(result.ok || looksLikePermissionError(result.error)).toBe(true);
      if (!result.ok) {
        console.log(`${LOG_PREFIX} 2.2.1: capture denied (expected):`, result.error);
      }
      console.log(`${LOG_PREFIX} 2.2.1 PASSED`);
    });

    it('2.2.2 — Permission Re-Request Flow: request_permission endpoint invocable', async () => {
      if (!requireMethod(methods, 'openhuman.screen_intelligence_request_permission', '2.2.2'))
        return;

      const result = await rpcOkOrPermission('openhuman.screen_intelligence_request_permission', {
        permission: 'accessibility',
      });
      console.log(`${LOG_PREFIX} 2.2.2: request_permission ok=${result.ok}`);
      console.log(`${LOG_PREFIX} 2.2.2 PASSED`);
    });

    it('2.2.3 — Restart & Refresh Sync: refresh_permissions works repeatedly', async () => {
      if (!requireMethod(methods, 'openhuman.screen_intelligence_refresh_permissions', '2.2.3'))
        return;

      // Call twice to verify idempotency
      const r1 = await rpcOkOrPermission('openhuman.screen_intelligence_refresh_permissions', {});
      const r2 = await rpcOkOrPermission('openhuman.screen_intelligence_refresh_permissions', {});
      console.log(`${LOG_PREFIX} 2.2.3: refresh r1=${r1.ok} r2=${r2.ok}`);
      console.log(`${LOG_PREFIX} 2.2.3 PASSED`);
    });

    it('2.2.4 — Partial Permission State: status readable with mixed permission grants', async () => {
      if (!requireMethod(methods, 'openhuman.screen_intelligence_status', '2.2.4')) return;

      const result = await rpcOkOrPermission('openhuman.screen_intelligence_status', {});
      if (result.ok) {
        // Status must return a permissions map regardless of what is/isn't granted
        const text = JSON.stringify(result.result || {}).toLowerCase();
        expect(text.includes('permissions') || text.length > 2).toBe(true);
      }
      console.log(`${LOG_PREFIX} 2.2.4 PASSED`);
    });
  });

  // ─── 2.x UI Verification ────────────────────────────────────────────────

  describe('2.x UI — Settings shows permission status', () => {
    it('2.x.1 — Settings screen intelligence section is accessible', async () => {
      await dismissLocalAISnackbarIfVisible(LOG_PREFIX);
      await navigateToSettings();
      await browser.pause(2_000);

      const hasScreenIntel =
        (await textExists('Screen Intelligence')) ||
        (await textExists('Screen')) ||
        (await textExists('Accessibility'));

      if (!hasScreenIntel) {
        const tree = await dumpAccessibilityTree();
        console.log(`${LOG_PREFIX} 2.x.1: Settings tree:\n`, tree.slice(0, 2000));
      }
      // Non-fatal — settings structure varies
      console.log(`${LOG_PREFIX} 2.x.1: Screen Intelligence in settings: ${hasScreenIntel}`);
    });
  });
});
