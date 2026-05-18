// @ts-nocheck
/**
 * Skill execution end-to-end (UI shell + core JSON-RPC runtime).
 *
 * Mirrors the Rust integration test
 * `json_rpc_skills_runtime_start_tools_call_stop` in
 * `tests/json_rpc_e2e.rs` — but goes through the same HTTP path the
 * desktop UI uses (`callOpenhumanRpc` → `http://127.0.0.1:<port>/rpc`).
 *
 * RPC result shapes:
 *   - skills_start              → SkillSnapshot ({ status, skill_id, … })
 *   - skills_call_tool          → ToolResult ({ content[] })
 *   - skills_stop               → { success, skill_id }
 *   - skills_set_setup_complete → ok / err
 *   - skills_status             → { setup_complete, … }
 *
 * Issue #68 (model → agent → tool → conversation) is environment- and
 * LLM-dependent; that's tracked separately. This spec validates the
 * skill runtime + RPC + Skills shell deterministically.
 */
import { waitForApp } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { dumpAccessibilityTree, textExists } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateToSkills } from '../helpers/shared-flows';
import {
  E2E_RUNTIME_SKILL_ID,
  removeSeededEchoSkill,
  seedMinimalEchoSkill,
} from '../helpers/skill-e2e-runtime';
import { getRequestLog, startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-skill-execution';

describe('Skill execution (UI + core RPC)', () => {
  before(async () => {
    await seedMinimalEchoSkill();
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
    await removeSeededEchoSkill();
  });

  it('lands the user on a logged-in shell', async () => {
    const atHome =
      (await textExists('Message OpenHuman')) ||
      (await textExists('Good morning')) ||
      (await textExists('Upgrade to Premium'));
    expect(atHome).toBe(true);
  });

  it('core.ping responds over the same JSON-RPC URL the UI uses', async () => {
    const ping = await callOpenhumanRpc('core.ping', {});
    expect(ping.ok).toBe(true);
  });

  it('runs start → list_tools → call_tool → stop for the seeded echo skill', async () => {
    const start = await callOpenhumanRpc('openhuman.skills_start', {
      skill_id: E2E_RUNTIME_SKILL_ID,
    });
    if (!start.ok) {
      console.error('[SkillExecutionE2E] skills_start failed', start, getRequestLog());
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
    expect(
      content.some(
        (c: { text?: string }) =>
          typeof c?.text === 'string' && c.text.includes('hello from e2e skill execution')
      )
    ).toBe(true);

    const stop = await callOpenhumanRpc('openhuman.skills_stop', {
      skill_id: E2E_RUNTIME_SKILL_ID,
    });
    expect(stop.ok).toBe(true);
    expect(stop.result?.success === true).toBe(true);
  });

  it('persists setup_complete via skills_set_setup_complete (OAuth path)', async () => {
    try {
      const set = await callOpenhumanRpc('openhuman.skills_set_setup_complete', {
        skill_id: E2E_RUNTIME_SKILL_ID,
        complete: true,
      });
      expect(set.ok).toBe(true);

      const st = await callOpenhumanRpc('openhuman.skills_status', {
        skill_id: E2E_RUNTIME_SKILL_ID,
      });
      expect(st.ok).toBe(true);
      expect(st.result?.setup_complete === true).toBe(true);
    } finally {
      await callOpenhumanRpc('openhuman.skills_set_setup_complete', {
        skill_id: E2E_RUNTIME_SKILL_ID,
        complete: false,
      });
    }
  });

  it('Skills UI surface shows installed tools', async () => {
    await navigateToSkills();
    await browser.pause(2_000);

    const hash = await browser.execute(() => window.location.hash);
    expect(String(hash)).toContain('/skills');

    const visible =
      (await textExists('Skills')) ||
      (await textExists('Install')) ||
      (await textExists('Available')) ||
      (await textExists('Telegram')) ||
      (await textExists('Notion'));
    if (!visible) {
      await dumpAccessibilityTree();
      console.error('[SkillExecutionE2E] request log:', getRequestLog());
    }
    expect(visible).toBe(true);
  });

  it.skip('(future) agent chat issues model tool_calls to echo — needs LLM + mock tool_calls', async () => {
    // Tracked under #68: drive chat with a prompt that forces tool use and assert echo in thread.
  });
});
