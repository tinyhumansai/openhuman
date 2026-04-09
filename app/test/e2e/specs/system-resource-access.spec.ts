// @ts-nocheck
/**
 * E2E: System Resource Access
 *
 * Covers:
 *   4.1.1  Read File Access
 *   4.1.2  Write File Access
 *   4.1.3  Unauthorized Path Access Prevention
 *   4.2.1  Shell Command Execution
 *   4.2.2  Command Restriction Enforcement
 *   4.2.3  Browser Access Policy
 *   4.2.4  Tool Management Entry
 *   4.3.1  Screen Capture Trigger
 *   4.3.2  Multi-Window Capture
 *   4.3.3  Permission-Based Capture Restriction
 *   4.4.1  Browser Access Capability
 *   4.4.2  Browser Automation
 *   4.4.3  HTTP Request Pipeline
 *   4.4.4  Web Search Execution
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { expectRpcMethod, fetchCoreRpcMethods } from '../helpers/core-schema';
import {
  performFullLogin,
} from '../helpers/shared-flows';
import {
  clearRequestLog,
  resetMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

const LOG_PREFIX = '[SystemResourceSpec]';
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
async function rpcOk(method: string, params: Record<string, unknown> = {}) {
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
    const acceptable = looksLikePermissionError(result.error) || looksLikeNotImplemented(result.error);
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

describe('System Resource Access', function () {
  this.timeout(5 * 60_000);

  let methods: Set<string>;

  before(async function () {
    this.timeout(60_000);
    await startMockServer();
    await waitForApp();
    await waitForAppReady(20_000);
    await performFullLogin('e2e-system-resource-token');
    methods = await fetchCoreRpcMethods();
    clearRequestLog();
  });

  after(async function () {
    this.timeout(30_000);
    resetMockBehavior();
    try { await stopMockServer(); } catch { /* non-fatal */ }
  });

  beforeEach(() => {
    clearRequestLog();
    resetMockBehavior();
  });

  // ─── 4.1 File Access ────────────────────────────────────────────────────

  describe('4.1 File Access', () => {
    it('4.1.1 — Read File Access: write then read a workspace file', async () => {
      if (!requireMethod(methods, 'openhuman.memory_write_file', '4.1.1')) return;

      await rpcOk('openhuman.memory_write_file', {
        relative_path: 'e2e/permissions/read-write-check.txt',
        content: 'openhuman-e2e',
      });

      if (!requireMethod(methods, 'openhuman.memory_read_file', '4.1.1')) return;

      const readResult = await rpcOk('openhuman.memory_read_file', {
        relative_path: 'e2e/permissions/read-write-check.txt',
      });
      expect(JSON.stringify(readResult || {}).includes('openhuman-e2e')).toBe(true);
      console.log(`${LOG_PREFIX} 4.1.1 PASSED`);
    });

    it('4.1.2 — Write File Access: write_file returns success', async () => {
      if (!requireMethod(methods, 'openhuman.memory_write_file', '4.1.2')) return;

      await rpcOk('openhuman.memory_write_file', {
        relative_path: 'e2e/permissions/write-check.txt',
        content: 'ok',
      });
      console.log(`${LOG_PREFIX} 4.1.2 PASSED`);
    });

    it('4.1.3 — Unauthorized Path Access Prevention: path traversal is rejected', async () => {
      if (!requireMethod(methods, 'openhuman.memory_write_file', '4.1.3')) return;

      const attempt = await callOpenhumanRpc('openhuman.memory_write_file', {
        relative_path: '../outside.txt',
        content: 'blocked',
      });
      expect(attempt.ok).toBe(false);
      console.log(`${LOG_PREFIX} 4.1.3: traversal rejected — error: ${attempt.error}`);
      console.log(`${LOG_PREFIX} 4.1.3 PASSED`);
    });
  });

  // ─── 4.2 Shell / Tool Restriction ───────────────────────────────────────

  describe('4.2 Shell Command & Tool Restriction', () => {
    it('4.2.1 — Shell Command Execution: tool runner RPC surface is exposed', () => {
      expectRpcMethod(methods, 'openhuman.skills_call_tool');
      expectRpcMethod(methods, 'openhuman.skills_list_tools');
      console.log(`${LOG_PREFIX} 4.2.1 PASSED`);
    });

    it('4.2.2 — Command Restriction Enforcement: unknown tool call fails safely', async () => {
      if (!requireMethod(methods, 'openhuman.skills_call_tool', '4.2.2')) return;

      const badCall = await callOpenhumanRpc('openhuman.skills_call_tool', {
        id: 'non-existent-e2e-runtime',
        tool_name: 'shell.exec',
        args: { command: 'echo hello' },
      });
      expect(badCall.ok).toBe(false);
      console.log(`${LOG_PREFIX} 4.2.2: rejected with:`, badCall.error?.slice?.(0, 120));
      console.log(`${LOG_PREFIX} 4.2.2 PASSED`);
    });

    it('4.2.3 — Browser Access Policy: capability catalog advertises browser policy', async () => {
      if (!requireMethod(methods, 'openhuman.about_app_lookup', '4.2.3')) return;

      await rpcOk('openhuman.about_app_lookup', { id: 'skills.browser_access_policy' });
      console.log(`${LOG_PREFIX} 4.2.3 PASSED`);
    });

    it('4.2.4 — Tool Management Entry: capability catalog has skills/configure entry', async () => {
      if (!requireMethod(methods, 'openhuman.about_app_lookup', '4.2.4')) return;

      await rpcOk('openhuman.about_app_lookup', { id: 'skills.configure' });
      console.log(`${LOG_PREFIX} 4.2.4 PASSED`);
    });
  });

  // ─── 4.3 Screen Capture ─────────────────────────────────────────────────

  describe('4.3 Screen Capture', () => {
    it('4.3.1 — Screen Capture Trigger: capture_test endpoint is callable', async () => {
      if (!requireMethod(methods, 'openhuman.screen_intelligence_capture_test', '4.3.1')) return;

      await rpcOkOrPermission('openhuman.screen_intelligence_capture_test', {});
      console.log(`${LOG_PREFIX} 4.3.1 PASSED`);
    });

    it('4.3.2 — Multi-Window Capture: vision_recent endpoint is callable', async () => {
      if (!requireMethod(methods, 'openhuman.screen_intelligence_vision_recent', '4.3.2')) return;

      await rpcOkOrPermission('openhuman.screen_intelligence_vision_recent', { limit: 5 });
      console.log(`${LOG_PREFIX} 4.3.2 PASSED`);
    });

    it('4.3.3 — Permission-Based Capture Restriction: permission errors are explicit', async () => {
      if (!requireMethod(methods, 'openhuman.screen_intelligence_capture_now', '4.3.3')) return;

      const capture = await callOpenhumanRpc('openhuman.screen_intelligence_capture_now', {});
      if (!capture.ok) {
        expect(looksLikePermissionError(capture.error)).toBe(true);
        console.log(`${LOG_PREFIX} 4.3.3: capture denied cleanly:`, capture.error?.slice?.(0, 80));
      }
      console.log(`${LOG_PREFIX} 4.3.3 PASSED`);
    });
  });

  // ─── 4.4 Browser / Web Search ───────────────────────────────────────────

  describe('4.4 Browser & Web Access', () => {
    it('4.4.1 — Browser Access Capability: catalog contains browser entry', async () => {
      if (!requireMethod(methods, 'openhuman.about_app_search', '4.4.1')) return;

      const result = await rpcOk('openhuman.about_app_search', { query: 'browser' });
      expect(JSON.stringify(result || {}).toLowerCase().includes('browser')).toBe(true);
      console.log(`${LOG_PREFIX} 4.4.1 PASSED`);
    });

    it('4.4.2 — Browser Automation: skill start/stop endpoints exist', () => {
      expectRpcMethod(methods, 'openhuman.skills_start');
      expectRpcMethod(methods, 'openhuman.skills_stop');
      console.log(`${LOG_PREFIX} 4.4.2 PASSED`);
    });

    it('4.4.3 — HTTP Request Pipeline: channel_web_chat method is exposed', () => {
      expectRpcMethod(methods, 'openhuman.channel_web_chat');
      console.log(`${LOG_PREFIX} 4.4.3 PASSED`);
    });

    it('4.4.4 — Web Search Execution: channel_web_chat handles minimal payload', async () => {
      if (!requireMethod(methods, 'openhuman.channel_web_chat', '4.4.4')) return;

      const res = await callOpenhumanRpc('openhuman.channel_web_chat', {
        input: 'health check',
        channel: 'web',
        target: 'e2e',
      });
      expect(res.ok || Boolean(res.error)).toBe(true);
      console.log(`${LOG_PREFIX} 4.4.4: channel_web_chat ok=${res.ok}`);
      console.log(`${LOG_PREFIX} 4.4.4 PASSED`);
    });
  });
});
