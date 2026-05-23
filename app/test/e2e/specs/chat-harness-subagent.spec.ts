/**
 * Chat harness — orchestrator → subagent flow.
 *
 * The default chat agent after onboarding is the **orchestrator**
 * (`src/openhuman/channels/providers/web.rs::pick_target_agent_id`).
 * Its `subagents = [...]` list synthesises one delegated archetype tool
 * per archetype at build time (see
 * `src/openhuman/tools/orchestrator_tools.rs`). When the LLM calls
 * `research` (or any other delegated archetype tool), the tool dispatches
 * to a sub-agent which runs the agent harness loop a level deeper —
 * which means the LLM gets hit at least once more for the sub-agent.
 *
 * What this spec scripts and verifies:
 *
 *   1. Configure `llmForcedResponses` with THREE responses in order:
 *        A) orchestrator turn — emits `research` tool_call
 *        B) researcher turn   — answers with a plain text finding
 *        C) orchestrator turn — final synthesis text (canary marker)
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
const PROMPT = 'Research the answer to life and tell me a marker phrase.';
const CANARY_FINAL = 'subagent-canary-final-7afe2';
const RESEARCHER_REPLY = 'The researcher answer is 42.';

// Three forced responses, popped in order by the mock LLM streamer.
const FORCED_RESPONSES = [
  // 1. Orchestrator: emit a research tool call.
  {
    content: '',
    toolCalls: [
      {
        id: 'call_research_1',
        name: 'research',
        arguments: JSON.stringify({ prompt: 'Tell me a marker phrase' }),
      },
    ],
  },
  // 2. Researcher sub-agent: produces a text answer.
  { content: RESEARCHER_REPLY },
  // 3. Orchestrator final synthesis containing the canary.
  { content: `Done. The result is: ${CANARY_FINAL}` },
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

describe('Chat harness — orchestrator → subagent flow', () => {
  before(async function beforeSuite() {
    this.timeout(90_000);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);

    setMockBehavior('llmForcedResponses', JSON.stringify(FORCED_RESPONSES));
    // Faster streaming for non-tool-call responses so this spec doesn't
    // need 30s of patience for three full streams.
    setMockBehavior('llmStreamChunkDelayMs', '10');
  });

  after(async () => {
    setMockBehavior('llmForcedResponses', '');
    setMockBehavior('llmStreamChunkDelayMs', '');
    await stopMockServer();
  });

  it('orchestrator delegates to researcher and produces the final canary', async function () {
    this.timeout(90_000);
    await navigateViaHash('/chat');
    await browser.waitUntil(async () => await textExists('Threads'), {
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
      if (await textExists(CANARY_FINAL)) break;
      await browser.pause(200);
    }

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
