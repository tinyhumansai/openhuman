// @ts-nocheck
/**
 * End-to-end: core JSON-RPC skill runtime (UI WebView → HTTP POST to sidecar) plus Skills UI smoke.
 * Mirrors the Rust integration test `json_rpc_skills_runtime_start_tools_call_stop` (tests/json_rpc_e2e.rs).
 *
 * Covers issue #68 acceptance: model/tool execution path is exercised via the same RPC surface the UI uses
 * (`callCoreRpc` / sidecar), with a deterministic echo tool — no silent RPC failures.
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  dumpAccessibilityTree,
  textExists,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { supportsExecuteScript } from '../helpers/platform';
import { completeOnboardingIfVisible, navigateToSkills } from '../helpers/shared-flows';
import { E2E_RUNTIME_SKILL_ID, seedMinimalEchoSkill } from '../helpers/skill-e2e-runtime';
import { clearRequestLog, getRequestLog, startMockServer, stopMockServer } from '../mock-server';

function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[SkillExecutionE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[SkillExecutionE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

describe('Skill execution (UI + core RPC)', () => {
  before(async () => {
    stepLog('Seeding minimal echo skill on disk');
    await seedMinimalEchoSkill();
    await startMockServer();
    await waitForApp();
    clearRequestLog();
  });

  after(async () => {
    await stopMockServer();
  });

  it('authenticates and reaches a logged-in shell', async () => {
    await triggerAuthDeepLinkBypass('e2e-skill-execution-token');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible('[SkillExecutionE2E]');
    const atHome =
      (await textExists('Message OpenHuman')) ||
      (await textExists('Good morning')) ||
      (await textExists('Upgrade to Premium'));
    expect(atHome).toBe(true);
  });

  it('core.ping responds over the same JSON-RPC URL as the UI', async () => {
    const ping = await callOpenhumanRpc('core.ping', {});
    if (!ping.ok) {
      stepLog('core.ping failed', ping);
    }
    expect(ping.ok).toBe(true);
  });

  it('runs start → list_tools → call_tool → stop for the seeded echo skill', async () => {
    const start = await callOpenhumanRpc('openhuman.skills_start', {
      skill_id: E2E_RUNTIME_SKILL_ID,
    });
    if (!start.ok) {
      stepLog('skills_start failed', start);
      stepLog('Request log (mock API):', getRequestLog());
    }
    expect(start.ok).toBe(true);
    const status = start.result?.status;
    expect(status === 'running' || status === 'initializing').toBe(true);

    await browser.pause(800);

    const tools = await callOpenhumanRpc('openhuman.skills_list_tools', {
      skill_id: E2E_RUNTIME_SKILL_ID,
    });
    expect(tools.ok).toBe(true);
    const toolNames = (tools.result?.tools || []).map((t: { name?: string }) => t.name);
    expect(toolNames.includes('echo')).toBe(true);

    const call = await callOpenhumanRpc('openhuman.skills_call_tool', {
      skill_id: E2E_RUNTIME_SKILL_ID,
      tool_name: 'echo',
      arguments: { message: 'hello from e2e skill execution' },
    });
    expect(call.ok).toBe(true);
    const content = call.result?.content || [];
    const echoed = content.some(
      (c: { text?: string }) =>
        typeof c?.text === 'string' && c.text.includes('hello from e2e skill execution')
    );
    expect(echoed).toBe(true);

    const stop = await callOpenhumanRpc('openhuman.skills_stop', {
      skill_id: E2E_RUNTIME_SKILL_ID,
    });
    expect(stop.ok).toBe(true);
    expect(stop.result?.success === true).toBe(true);
  });

  it('Skills page loads (UI surface for installed tools)', async () => {
    await navigateToSkills();
    await browser.pause(2_000);
    if (supportsExecuteScript()) {
      const hash = await browser.execute(() => window.location.hash);
      expect(String(hash)).toContain('/skills');
    }

    const visible =
      (await textExists('Skills')) ||
      (await textExists('Install')) ||
      (await textExists('Available')) ||
      (await textExists('Telegram')) ||
      (await textExists('Notion'));
    if (!visible) {
      stepLog('Skills markers missing');
      await dumpAccessibilityTree();
      stepLog('Request log:', getRequestLog());
    }
    expect(visible).toBe(true);
  });
});
