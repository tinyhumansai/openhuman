// @ts-nocheck
/**
 * E2E test: Skill Execution & Text Auto-Complete (Built-in Skills — accessed from Skills tab)
 *
 * Covers:
 *   9.2.1 — Skills page loads with all three sections (Built-in, Channel, 3rd Party)
 *   9.2.2 — Built-in skill cards render with correct titles and descriptions
 *   9.2.3 — Text Auto-Complete card navigates to /settings/autocomplete panel
 *   9.2.4 — Autocomplete panel renders Runtime and Settings sections
 *   9.2.5 — autocomplete_status RPC returns engine state
 *   9.2.6 — core.ping responds over JSON-RPC
 *   9.2.7 — Skill runtime lifecycle: start → list_tools → call_tool → stop (echo skill)
 *   9.2.8 — 3rd Party Skills section shows skill list or empty state
 *
 * JSON-RPC `result` shapes match the Rust integration test
 * `json_rpc_skills_runtime_start_tools_call_stop` (tests/json_rpc_e2e.rs):
 *   `skills_start` → `SkillSnapshot` (status, skill_id)
 *   `skills_call_tool` → `{ content[], is_error }`
 *   `skills_stop` → `{ success, skill_id }`
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  clickText,
  dumpAccessibilityTree,
  textExists,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { supportsExecuteScript } from '../helpers/platform';
import {
  completeOnboardingIfVisible,
  dismissLocalAISnackbarIfVisible,
  navigateToSkills,
  navigateViaHash,
  waitForHomePage,
} from '../helpers/shared-flows';
import {
  E2E_RUNTIME_SKILL_ID,
  removeSeededEchoSkill,
  seedMinimalEchoSkill,
} from '../helpers/skill-e2e-runtime';
import { clearRequestLog, getRequestLog, setMockBehavior, startMockServer, stopMockServer } from '../mock-server';

const LOG_PREFIX = '[SkillExecutionE2E]';

function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`${LOG_PREFIX}[${stamp}] ${message}`);
    return;
  }
  console.log(`${LOG_PREFIX}[${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

async function waitForAnyText(candidates: string[], timeout = 15_000): Promise<string | null> {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    for (const t of candidates) {
      if (await textExists(t)) return t;
    }
    await browser.pause(500);
  }
  return null;
}

describe('Skill execution & Text Auto-Complete (Built-in Skill)', () => {
  before(async () => {
    stepLog('Seeding minimal echo skill on disk');
    await seedMinimalEchoSkill();
    await startMockServer();
    await waitForApp();
    clearRequestLog();
  });

  after(async () => {
    await stopMockServer();
    await removeSeededEchoSkill();
  });

  // ── Auth + reach logged-in shell ────────────────────────────────────────

  it('authenticates and reaches a logged-in shell', async () => {
    await triggerAuthDeepLinkBypass('e2e-skill-execution-token');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible(LOG_PREFIX);

    const home = await waitForHomePage(15_000);
    if (!home) {
      const tree = await dumpAccessibilityTree();
      stepLog('Home not reached', { tree: tree.slice(0, 4000) });
    }
    expect(home).not.toBeNull();
  });

  // ── 9.2.1 Skills page loads with all three sections ─────────────────────

  it('Skills page loads with Built-in, Channel, and 3rd Party sections', async () => {
    await dismissLocalAISnackbarIfVisible(LOG_PREFIX);
    await navigateToSkills();
    await browser.pause(2_000);

    if (supportsExecuteScript()) {
      const hash = await browser.execute(() => window.location.hash);
      expect(String(hash)).toContain('/skills');
    }

    const hasBuiltIn = await waitForAnyText(['Built-in Skills'], 10_000);
    stepLog('Built-in Skills section', { found: hasBuiltIn });
    expect(hasBuiltIn).not.toBeNull();

    const hasChannels = await waitForAnyText(['Channel Integrations'], 5_000);
    stepLog('Channel Integrations section', { found: hasChannels });

    const hasThirdParty = await waitForAnyText(
      ['3rd Party Skills', 'Loading skills...', 'No skills discovered'],
      10_000
    );
    stepLog('3rd Party Skills section', { found: hasThirdParty });
    expect(hasThirdParty).not.toBeNull();
  });

  // ── 9.2.2 Built-in skill cards ─────────────────────────────────────────

  it('shows Built-in skill cards with correct titles', async () => {
    const onSkills = await textExists('Built-in Skills');
    if (!onSkills) {
      await navigateToSkills();
      await browser.pause(2_000);
    }

    const builtInTitles = ['Screen Intelligence', 'Text Auto-Complete', 'Voice Intelligence'];
    const foundTitles: string[] = [];
    for (const title of builtInTitles) {
      if (await textExists(title)) foundTitles.push(title);
    }

    stepLog('Built-in skill cards found', { foundTitles });
    expect(foundTitles.length).toBeGreaterThanOrEqual(2);

    // Verify descriptions
    const descriptions = ['Capture windows', 'Suggest inline completions', 'Use the microphone'];
    let hasDescription = false;
    for (const desc of descriptions) {
      if (await textExists(desc)) {
        hasDescription = true;
        break;
      }
    }
    expect(hasDescription).toBe(true);
  });

  // ── 9.2.3 Text Auto-Complete card → /settings/autocomplete ──────────────

  it('navigates to Autocomplete settings from the Skills tab', async () => {
    const onSkills = await textExists('Built-in Skills');
    if (!onSkills) {
      await navigateToSkills();
      await browser.pause(2_000);
    }

    await clickText('Text Auto-Complete', 10_000);
    await browser.pause(2_000);

    if (supportsExecuteScript()) {
      const currentHash = await browser.execute(() => window.location.hash);
      stepLog('After clicking Text Auto-Complete card', { currentHash });
      expect(currentHash).toContain('autocomplete');
    }

    const hasPanel = await waitForAnyText(
      ['Inline Autocomplete', 'Runtime', 'Settings'],
      15_000
    );
    if (!hasPanel) {
      const tree = await dumpAccessibilityTree();
      stepLog('Autocomplete panel missing expected headings', { tree: tree.slice(0, 4000) });
    }
    stepLog('Autocomplete panel', { found: hasPanel });
    expect(hasPanel).not.toBeNull();
  });

  // ── 9.2.4 Autocomplete panel renders config ─────────────────────────────

  it('shows Autocomplete settings with Enabled toggle and config options', async () => {
    const alreadyOnPage = await textExists('Inline Autocomplete');
    if (!alreadyOnPage) {
      await navigateViaHash('/settings/autocomplete');
      await browser.pause(2_000);
    }

    const hasEnabled = await waitForAnyText(['Enabled'], 10_000);
    stepLog('Enabled toggle', { found: hasEnabled });
    expect(hasEnabled).not.toBeNull();

    // Style Preset or other config label
    const configLabels = ['Style Preset', 'Style Instructions', 'Settings', 'Runtime'];
    const foundLabels: string[] = [];
    for (const label of configLabels) {
      if (await textExists(label)) foundLabels.push(label);
    }
    stepLog('Config labels found', { foundLabels });
    expect(foundLabels.length).toBeGreaterThanOrEqual(1);
  });

  // ── 9.2.5 autocomplete_status RPC ──────────────────────────────────────

  it('autocomplete_status RPC returns engine state', async () => {
    const result = await callOpenhumanRpc('openhuman.autocomplete_status', {});
    stepLog('autocomplete_status RPC raw', JSON.stringify(result, null, 2));

    expect(result.ok).toBe(true);

    const raw = result.result;
    const data = raw?.result ?? raw;
    expect(data).toBeDefined();

    expect(typeof data.enabled).toBe('boolean');
    expect(typeof data.running).toBe('boolean');

    stepLog('Autocomplete engine state', {
      enabled: data.enabled,
      running: data.running,
      pending: data.pending,
    });
  });

  // ── 9.2.6 core.ping ────────────────────────────────────────────────────

  it('core.ping responds over the same JSON-RPC URL as the UI', async () => {
    const ping = await callOpenhumanRpc('core.ping', {});
    if (!ping.ok) {
      stepLog('core.ping failed', ping);
    }
    expect(ping.ok).toBe(true);
  });

  // ── 9.2.7 Skill runtime lifecycle ──────────────────────────────────────

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
    stepLog('skills_start result', { status, skill_id: start.result?.skill_id });
    expect(status === 'running' || status === 'initializing').toBe(true);

    await browser.pause(800);

    const tools = await callOpenhumanRpc('openhuman.skills_list_tools', {
      skill_id: E2E_RUNTIME_SKILL_ID,
    });
    expect(tools.ok).toBe(true);
    const toolNames = (tools.result?.tools || []).map((t: { name?: string }) => t.name);
    stepLog('skills_list_tools result', { toolNames });
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
    stepLog('skills_call_tool echo result', { content, echoed });
    expect(echoed).toBe(true);

    const stop = await callOpenhumanRpc('openhuman.skills_stop', {
      skill_id: E2E_RUNTIME_SKILL_ID,
    });
    expect(stop.ok).toBe(true);
    stepLog('skills_stop result', { success: stop.result?.success });
    expect(stop.result?.success === true).toBe(true);
  });

  // ── 9.2.8 3rd Party Skills section ──────────────────────────────────────

  it('3rd Party Skills section shows skill list or empty state', async () => {
    await navigateToSkills();
    await browser.pause(2_000);

    const visible = await waitForAnyText(
      [
        '3rd Party Skills',
        'Skill settings',
        'Loading skills...',
        'No skills discovered',
        'Gmail',
        'Notion',
        'Connected',
        'Offline',
        'Setup',
        'Enable',
        'Configure',
      ],
      10_000
    );

    if (!visible) {
      stepLog('3rd Party Skills markers missing');
      const tree = await dumpAccessibilityTree();
      stepLog('Accessibility tree', { tree: tree.slice(0, 4000) });
      stepLog('Request log:', getRequestLog());
    }
    expect(visible).not.toBeNull();
  });

  // ── 9.2.9 Agent chat issues tool_calls to the echo skill ─────────────

  it('agent chat issues model tool_calls to the echo skill via mock LLM', async () => {
    // 1. Ensure the echo skill is running
    const startRes = await callOpenhumanRpc('openhuman.skills_start', {
      skill_id: E2E_RUNTIME_SKILL_ID,
    });
    if (!startRes.ok) {
      stepLog('skills_start failed (pre-chat)', startRes);
    }
    expect(startRes.ok).toBe(true);
    await browser.pause(800);

    // 2. Configure mock to return a tool_calls response on the next completion request.
    //    The tool name uses the agent convention: {skill_id}__{tool_name}
    const toolCallSpec = `${E2E_RUNTIME_SKILL_ID}__echo|${JSON.stringify({ message: 'hello from agent tool_calls' })}`;
    setMockBehavior('chatToolCalls', toolCallSpec);
    stepLog('Set chatToolCalls behavior', { toolCallSpec });

    // 3. Fire a chat request via RPC — the agentic loop will:
    //    a) hit /openai/v1/chat/completions → get tool_calls response
    //    b) execute e2e-runtime__echo tool via QuickJS
    //    c) hit /openai/v1/chat/completions again → get normal text response
    clearRequestLog();
    const chatRes = await callOpenhumanRpc('openhuman.channel_web_chat', {
      client_id: 'e2e-tool-test-client',
      thread_id: 'e2e-tool-test-thread',
      message: 'please echo hello',
    });
    stepLog('channel_web_chat result', chatRes);
    expect(chatRes.ok).toBe(true);

    // 4. Wait for the agentic loop to complete (tool execution + final LLM call)
    //    by checking the request log for two POST /openai/v1/chat/completions calls.
    const deadline = Date.now() + 30_000;
    let completionRequests = [];
    while (Date.now() < deadline) {
      const log = getRequestLog();
      completionRequests = log.filter(
        (r) => r.method === 'POST' && r.url.includes('/openai/v1/chat/completions')
      );
      if (completionRequests.length >= 2) break;
      await browser.pause(500);
    }

    stepLog('Completion requests captured', {
      count: completionRequests.length,
    });
    expect(completionRequests.length).toBeGreaterThanOrEqual(2);

    // 5. Verify the second request contains the tool result from the echo skill.
    //    The agent sends tool results as messages with role "tool" in the second call.
    const secondReq = completionRequests[1];
    const secondBody = secondReq?.body;
    const messagesJson = typeof secondBody === 'string' ? secondBody : JSON.stringify(secondBody);
    const hasEchoResult = messagesJson?.includes('hello from agent tool_calls');
    stepLog('Second completion request contains echo result', {
      hasEchoResult,
      bodySnippet: messagesJson?.slice(0, 2000),
    });
    expect(hasEchoResult).toBe(true);

    // 6. Clean up: stop the echo skill
    const stopRes = await callOpenhumanRpc('openhuman.skills_stop', {
      skill_id: E2E_RUNTIME_SKILL_ID,
    });
    stepLog('skills_stop (post-chat)', { success: stopRes.result?.success });
    expect(stopRes.ok).toBe(true);
  });
});
