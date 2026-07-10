/**
 * Chat harness — orchestrator → subagent flow.
 *
 * The default chat agent after onboarding is the **orchestrator**
 * (`src/openhuman/channels/providers/web.rs::pick_target_agent_id`).
 * Its `[subagents] allowlist = [...]` section synthesises one delegated archetype tool
 * per archetype at build time (see
 * `src/openhuman/tools/orchestrator_tools.rs`). When the LLM calls
 * `research` (or any other delegated archetype tool), the tool dispatches
 * to a sub-agent which runs the agent harness loop a level deeper —
 * which means the LLM gets hit at least once more for the sub-agent.
 *
 * Scripting: `llmKeywordRules` (content-addressed, never depleted).
 * A prior version of this spec used the `llmForcedResponses` FIFO but
 * that queue drains one entry per `/chat/completions` regardless of who
 * called (#4517): the sub-agent's own harness loop plus any ancillary
 * summarisation/memory-prep call shifts responses out of order and the
 * scripted final canary lands on the wrong turn (or never renders).
 * Keyword rules route each call by a substring of its latest
 * user/tool message, so extra calls that don't match any rule fall
 * through to the mock's dynamic default — the scripted turns are never
 * consumed by an off-turn caller.
 *
 * What this spec scripts and verifies:
 *
 *   1. Configure `llmKeywordRules` with three rules keyed on
 *      per-turn-unique tokens:
 *        A) orchestrator turn (user PROMPT)   — emits `research` tool_call
 *        B) researcher turn (delegate prompt) — plain text finding
 *        C) orchestrator turn (tool result)   — final synthesis (canary)
 *
 *   2. Send the user prompt and watch the runtime:
 *        UI:
 *          - The redux `chatRuntime.inferenceStatusByThread[<thread>]`
 *            transitions through `phase: 'subagent'` at some point.
 *          - `chatRuntime.toolTimelineByThread[<thread>]` records an
 *            entry whose `id` starts with `<thread>:subagent:`.
 *          - The final orchestrator text (canary) renders in the DOM.
 *
 *        Rust:
 *          - IN_FLIGHT clears after `chat_done`.
 *
 *        Mock backend:
 *          - The mock LLM received at least 2 POSTs to
 *            `/openai/v1/chat/completions` (orchestrator + sub-agent).
 *
 *        Workspace:
 *          - The persisted thread JSONL contains the final canary text.
 */
import { waitForApp } from '../helpers/app-helpers';
import {
  chatMounted,
  clickByTitle,
  clickSend,
  getSelectedThreadId,
  hexEncodeThreadId,
  typeIntoComposer,
  waitForSocketConnected,
} from '../helpers/chat-harness';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { textExists } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { getRequestLog, setMockBehavior, startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-chat-harness-subagent';
// Per-turn tokens chosen so pickProbeText's substring match routes each
// call to exactly one rule regardless of any extra ancillary
// tool/context/summary call the harness may issue on top of the three
// happy-path turns.
const PROMPT = 'Please delegate a llama-research task and return the marker.';
const DELEGATE_PROMPT = 'Return the coded marker phrase.';
const RESEARCHER_REPLY = 'The researcher trace signal is FORTY-TWO.';
const CANARY_FINAL = 'subagent-canary-final-7afe2';

// Content-addressed keyword rules — never depleted, immune to extra
// ancillary /chat/completions calls (#4517).
const KEYWORD_RULES = [
  // Sub-agent turn — dispatch_subagent renders the arg as "Task:\n<arg>"
  // (archetype_delegation.rs render_structured_handoff), so DELEGATE_PROMPT
  // surfaces verbatim in the researcher's user message.
  { keyword: 'coded marker phrase', content: RESEARCHER_REPLY },
  // Orchestrator's post-delegation turn — the sub-agent's output is handed
  // back as the `role: tool` message content
  // (dispatch.rs `ToolResult::success(outcome.output)`).
  { keyword: 'researcher trace signal', content: `Done. The result is: ${CANARY_FINAL}` },
  // Orchestrator's initial turn — the fire-and-forget thread-title-gen
  // call (threadSlice.ts, tools: None) sees the same probe, but
  // chat_with_system consumes `content` and ignores unexpected tool_calls,
  // so a delegation-triggering rule here is safe for both callers.
  {
    keyword: 'llama-research',
    content: 'Delegating to researcher.',
    toolCalls: [
      {
        id: 'call_research_1',
        name: 'research',
        arguments: JSON.stringify({ prompt: DELEGATE_PROMPT }),
      },
    ],
  },
];

interface RuntimeSnapshot {
  phase?: string;
  activeSubagent?: string;
  timelineIds: string[];
  timelineNames: string[];
}

async function snapshotRuntime(threadId: string): Promise<RuntimeSnapshot> {
  return (await browser.execute((tid: string) => {
    const winAny = window as unknown as { __OPENHUMAN_STORE__?: { getState: () => unknown } };
    const state = winAny.__OPENHUMAN_STORE__?.getState() as
      | {
          chatRuntime?: {
            inferenceStatusByThread?: Record<string, { phase?: string; activeSubagent?: string }>;
            toolTimelineByThread?: Record<string, Array<{ id?: string; name?: string }>>;
          };
        }
      | undefined;
    const status = state?.chatRuntime?.inferenceStatusByThread?.[tid];
    const timeline = state?.chatRuntime?.toolTimelineByThread?.[tid] ?? [];
    return {
      phase: status?.phase,
      activeSubagent: status?.activeSubagent,
      timelineIds: timeline.map(e => e?.id ?? ''),
      timelineNames: timeline.map(e => e?.name ?? ''),
    };
  }, threadId)) as RuntimeSnapshot;
}

async function hasRenderedSubagentTimeline(): Promise<boolean> {
  return (await browser.execute(() => {
    const rows = Array.from(document.querySelectorAll('[data-testid="agent-timeline-row"]'));
    return rows.some(row => {
      const text = row.textContent ?? '';
      return /Research|Researching|subagent/i.test(text);
    });
  })) as boolean;
}

describe('Chat harness — orchestrator → subagent flow', () => {
  before(async function beforeSuite() {
    this.timeout(90_000);
    await startMockServer();
    await waitForApp();
    // clearAuthSession drops any session token a prior chat-harness spec in
    // this shard left behind, so the orchestrator/sub-agent run starts from a
    // clean signed-in state rather than a polluted one (the source of the
    // intermittent "final canary never arrives" failures).
    await resetApp(USER_ID, { clearAuthSession: true });
    const superContext = await callOpenhumanRpc('openhuman.config_set_super_context_enabled', {
      value: false,
    });
    expect(superContext.ok).toBe(true);
    console.log(
      '[chat-harness-subagent] Disabled super context for deterministic scripted LLM calls'
    );

    setMockBehavior('llmKeywordRules', JSON.stringify(KEYWORD_RULES));
    // Faster streaming for non-tool-call responses so this spec doesn't
    // need 30s of patience for three full streams.
    setMockBehavior('llmStreamChunkDelayMs', '10');
  });

  after(async () => {
    setMockBehavior('llmKeywordRules', '');
    setMockBehavior('llmStreamChunkDelayMs', '');
    await stopMockServer();
  });

  it('orchestrator delegates to researcher and produces the final canary', async function () {
    this.timeout(90_000);
    await navigateViaHash('/chat');
    await browser.waitUntil(async () => await chatMounted(), {
      timeout: 15_000,
      timeoutMsg: 'Conversations did not mount',
    });
    expect(await clickByTitle('New thread', 8_000)).toBe(true);

    const threadId = (await browser.waitUntil(async () => await getSelectedThreadId(), {
      timeout: 8_000,
      timeoutMsg: 'thread.selectedThreadId never populated',
    })) as string;
    expect(typeof threadId).toBe('string');

    await typeIntoComposer(PROMPT);
    const socketReady = await waitForSocketConnected(30_000);
    if (!socketReady) {
      console.warn('[chat-harness-subagent] socket did not connect within 30 s — send may fail');
    }
    expect(
      await browser.waitUntil(async () => await clickSend(), {
        timeout: 5_000,
        timeoutMsg: 'Send button never enabled',
      })
    ).toBe(true);

    // Watch the runtime: at some point during the turn the phase
    // should flip to 'subagent' and a `subagent:researcher` timeline
    // entry should appear.
    let sawSubagentPhase = false;
    let sawSubagentTimeline = false;
    const deadline = Date.now() + 45_000;
    while (Date.now() < deadline) {
      const snap = await snapshotRuntime(threadId);
      if (snap.phase === 'subagent') sawSubagentPhase = true;
      if (
        snap.timelineIds.some(id => id.includes(':subagent:')) ||
        snap.timelineNames.some(n => n.startsWith('subagent:'))
      ) {
        sawSubagentTimeline = true;
      }
      if (sawSubagentPhase && sawSubagentTimeline) break;
      if (await textExists(CANARY_FINAL)) {
        sawSubagentTimeline = sawSubagentTimeline || (await hasRenderedSubagentTimeline());
        break;
      }
      await browser.pause(200);
    }

    sawSubagentTimeline = sawSubagentTimeline || (await hasRenderedSubagentTimeline());

    // At least ONE of the two signals must have fired — the timeline
    // entry is the more durable check (the live phase can flip back to
    // 'thinking' or 'idle' before our 200ms poll catches it).
    expect(sawSubagentPhase || sawSubagentTimeline).toBe(true);

    // Final canary must land in the DOM after the orchestrator wraps up.
    await browser.waitUntil(async () => await textExists(CANARY_FINAL), {
      timeout: 30_000,
      timeoutMsg: 'orchestrator never produced the final canary text',
    });

    // IN_FLIGHT must drain after chat_done.
    await browser.waitUntil(
      async () => {
        const snap = await callOpenhumanRpc<{ result: { entries: Array<unknown> } }>(
          'openhuman.test_support_in_flight_chats',
          {}
        );
        return snap.ok && (snap.result?.result?.entries?.length ?? 0) === 0;
      },
      { timeout: 10_000, timeoutMsg: 'IN_FLIGHT never cleared after orchestrator finished' }
    );
  });

  it('the mock LLM saw multiple chat-completions requests (parent + sub-agent)', async () => {
    const log = getRequestLog() as Array<{ method: string; url: string; body?: string }>;
    const llmHits = log.filter(
      r => r.method === 'POST' && r.url.includes('/openai/v1/chat/completions')
    );
    // Orchestrator turn 1 (emits tool_call) + sub-agent turn + orchestrator turn 2 = 3.
    // Accept ≥2 to stay robust against orchestrator-skipping or tool-loop
    // optimisations that fold the final synthesis into the tool response.
    expect(llmHits.length).toBeGreaterThanOrEqual(2);
  });

  it('persisted thread file records the final orchestrator text', async () => {
    const threadId = await getSelectedThreadId();
    expect(typeof threadId).toBe('string');
    const relPath = `memory/conversations/threads/${hexEncodeThreadId(threadId as string)}.jsonl`;

    let content = '';
    // The orchestrator's final synthesis may take extra time to persist:
    // the agent harness flushes the JSONL asynchronously after the stream
    // completes. Allow up to 30s for disk write to land.
    const deadline = Date.now() + 30_000;
    while (Date.now() < deadline) {
      const read = await callOpenhumanRpc<{ result: { content_utf8: string } }>(
        'openhuman.test_support_read_workspace_file',
        { rel_path: relPath, max_bytes: 131_072 }
      );
      if (read.ok && read.result?.result?.content_utf8) {
        content = read.result.result.content_utf8;
        if (content.includes(CANARY_FINAL)) break;
      }
      await browser.pause(500);
    }
    expect(content).toContain(CANARY_FINAL);
  });
});
