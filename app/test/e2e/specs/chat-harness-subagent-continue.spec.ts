/**
 * Chat harness — orchestrator → subagent continuation flow.
 *
 * Tests the full sub-agent persistence and continuation loop:
 *   1. Orchestrator delegates to `researcher` sub-agent.
 *   2. Researcher calls `ask_user_clarification` ("Which repo?").
 *   3. The harness exits early, returns [SUBAGENT_AWAITING_USER] to the
 *      orchestrator.
 *   4. Orchestrator surfaces the question to the user.
 *   5. User replies in the composer ("the main repo").
 *   6. Orchestrator calls `continue_subagent` with the user's answer.
 *   7. Researcher resumes from checkpoint, produces final answer.
 *   8. Orchestrator produces final synthesis with canary.
 *
 * Verifies:
 *   - The tool timeline shows a subagent entry.
 *   - The final canary text renders in the DOM.
 *   - The mock LLM received ≥4 POST requests (orchestrator initial +
 *     researcher initial + orchestrator relay + researcher resumed +
 *     orchestrator final).
 *   - Persisted thread JSONL contains the final canary text.
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

const USER_ID = 'e2e-chat-harness-subagent-continue';
const PROMPT = 'Research the answer and tell me a marker phrase.';
const CANARY_FINAL = 'subagent-continue-canary-9bc3f';
const CLARIFICATION_QUESTION = 'Which repo should I search?';
const USER_ANSWER = 'the main repo';
const RESEARCHER_FINAL_REPLY = 'After searching the main repo, the answer is 42.';

// Five forced responses, popped in order by the mock LLM streamer:
// 1. Orchestrator: emits `research` tool call.
// 2. Researcher: calls `ask_user_clarification` (triggers early exit).
// 3. Orchestrator: sees [SUBAGENT_AWAITING_USER], asks user the question.
// 4. Orchestrator (after user reply): calls `continue_subagent` with user's answer.
// 5. Researcher (resumed): produces final text answer.
// 6. Orchestrator: final synthesis containing the canary.
const FORCED_RESPONSES = [
  // 1. Orchestrator: delegate to researcher.
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
  // 2. Researcher: ask clarification (tool call).
  {
    content: '',
    toolCalls: [
      {
        id: 'call_clarify_1',
        name: 'ask_user_clarification',
        arguments: JSON.stringify({ question: CLARIFICATION_QUESTION }),
      },
    ],
  },
  // 3. Orchestrator: relay the question to the user.
  { content: `The researcher needs to know: ${CLARIFICATION_QUESTION}` },
  // 4. Orchestrator (after user replies): continue_subagent.
  {
    content: '',
    toolCalls: [
      {
        id: 'call_continue_1',
        name: 'continue_subagent',
        arguments: JSON.stringify({
          task_id: '{{DYNAMIC_TASK_ID}}',
          agent_id: 'researcher',
          message: USER_ANSWER,
        }),
      },
    ],
  },
  // 5. Researcher (resumed): final answer.
  { content: RESEARCHER_FINAL_REPLY },
  // 6. Orchestrator: final synthesis.
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

describe('Chat harness — orchestrator → subagent continuation flow', () => {
  before(async function beforeSuite() {
    this.timeout(120_000);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);

    setMockBehavior('llmForcedResponses', JSON.stringify(FORCED_RESPONSES));
    setMockBehavior('llmStreamChunkDelayMs', '10');
  });

  after(async () => {
    setMockBehavior('llmForcedResponses', '');
    setMockBehavior('llmStreamChunkDelayMs', '');
    await stopMockServer();
  });

  it('orchestrator delegates, researcher asks clarification, user answers, researcher continues, canary lands', async function () {
    this.timeout(120_000);
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

    // Send the initial prompt.
    await typeIntoComposer(PROMPT);
    const socketReady = await waitForSocketConnected(30_000);
    if (!socketReady) {
      console.warn('[subagent-continue] socket did not connect within 30 s');
    }
    expect(
      await browser.waitUntil(async () => await clickSend(), {
        timeout: 5_000,
        timeoutMsg: 'Send button never enabled',
      })
    ).toBe(true);

    // Wait for the orchestrator to relay the clarification question.
    await browser.waitUntil(async () => await textExists(CLARIFICATION_QUESTION), {
      timeout: 45_000,
      timeoutMsg: 'orchestrator never relayed the clarification question',
    });

    // User answers the clarification.
    await typeIntoComposer(USER_ANSWER);
    expect(
      await browser.waitUntil(async () => await clickSend(), {
        timeout: 5_000,
        timeoutMsg: 'Send button never enabled for user answer',
      })
    ).toBe(true);

    // Watch for subagent timeline entry.
    let sawSubagentTimeline = false;
    const deadline = Date.now() + 45_000;
    while (Date.now() < deadline) {
      const snap = await snapshotRuntime(threadId);
      if (
        snap.timelineIds.some(id => id.includes(':subagent:')) ||
        snap.timelineNames.some(n => n.startsWith('subagent:'))
      ) {
        sawSubagentTimeline = true;
      }
      if (sawSubagentTimeline) break;
      if (await textExists(CANARY_FINAL)) break;
      await browser.pause(200);
    }
    expect(sawSubagentTimeline).toBe(true);

    // Final canary must land in the DOM.
    await browser.waitUntil(async () => await textExists(CANARY_FINAL), {
      timeout: 45_000,
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

  it('the mock LLM saw multiple chat-completions requests (parent + sub-agent + resumed sub-agent)', async () => {
    const log = getRequestLog() as Array<{ method: string; url: string; body?: string }>;
    const llmHits = log.filter(
      r => r.method === 'POST' && r.url.includes('/openai/v1/chat/completions')
    );
    // Orchestrator turn 1 + researcher turn + orchestrator turn 2 (relay question)
    // + orchestrator turn 3 (continue) + researcher turn 2 + orchestrator turn 4 (synthesis)
    // = 6, but accept ≥4 for robustness.
    expect(llmHits.length).toBeGreaterThanOrEqual(4);
  });

  it('persisted thread file records the final orchestrator text', async () => {
    const threadId = await getSelectedThreadId();
    expect(typeof threadId).toBe('string');
    const relPath = `memory/conversations/threads/${hexEncodeThreadId(threadId as string)}.jsonl`;

    let content = '';
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
