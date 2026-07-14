/**
 * Chat harness — orchestrator → subagent continuation flow.
 *
 * Tests the full sub-agent persistence and continuation loop:
 *   1. Orchestrator delegates to `researcher` sub-agent.
 *   2. Researcher calls `ask_user_clarification` ("Which repo?").
 *   3. The harness exits early, returns [SUBAGENT_AWAITING_USER] to the
 *      orchestrator.
 *   4. Orchestrator surfaces the question to the user.
 *   5. User replies in the composer.
 *   6. Orchestrator calls `continue_subagent` with the user's answer.
 *   7. Researcher resumes from checkpoint, produces final answer.
 *   8. Orchestrator produces final synthesis with canary.
 *
 * Scripting: `llmKeywordRules` + mock `{{DYNAMIC_*}}` template substitution.
 * A prior version used `llmForcedResponses` but that FIFO drains one entry
 * per `/chat/completions` call regardless of who called; the researcher's
 * own harness loop plus any ancillary summarisation/memory-prep call
 * shifts responses out of order and the scripted final canary lands on
 * the wrong turn or never renders (tinyhumansai/openhuman#4517). Keyword
 * rules route each call by a substring of its latest user/tool message
 * and are never consumed.
 *
 * The orchestrator's `continue_subagent` call needs a real `task_id`
 * (`sub-<uuid>` generated at delegate time in
 * `src/openhuman/agent_orchestration/tools/dispatch.rs`). Tests can't
 * know it ahead of time, so the mock substitutes `{{DYNAMIC_TASK_ID}}` /
 * `{{DYNAMIC_AGENT_ID}}` from the latest `[SUBAGENT_AWAITING_USER]`
 * envelope in the message history (see
 * `scripts/mock-api/routes/llm/shared.mjs::renderDynamicPlaceholders`).
 *
 * Verifies:
 *   - The tool timeline shows a subagent entry.
 *   - The final canary text renders in the DOM.
 *   - The mock LLM received ≥4 POST requests (orchestrator initial +
 *     researcher initial + orchestrator relay + orchestrator continue +
 *     researcher resumed + orchestrator final).
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
// Per-turn probes carry disjoint distinctive tokens so `pickProbeText`'s
// substring match routes each of the six model calls to exactly one rule
// regardless of any ancillary summarisation / title-gen calls the harness may
// issue on top of the happy-path turns.
const PROMPT =
  'Please delegate a research task with the chromatophore-request badge and return a marker phrase.';
const DELEGATE_PROMPT = 'Return the harlequin-token phrase after searching.';
const CLARIFICATION_QUESTION = 'Which repo should I search for the citrine-cliff badge?';
const USER_ANSWER = 'Please try the wisteria-tag project.';
const RESEARCHER_FINAL_REPLY = 'In the wisteria-tag project I found the zenith-beacon marker.';
const CANARY_FINAL = 'subagent-continue-canary-9bc3f';

// Content-addressed keyword rules — never depleted, immune to extra
// ancillary /chat/completions calls (#4517).
//
// ORDER MATTERS: the researcher's resumed probe includes both the framing
// `[User's answer to your clarification question]` prepended by
// `src/openhuman/agent_orchestration/tools/continue_subagent.rs` AND the raw
// user answer text (`wisteria-tag`). Its rule must appear BEFORE the
// orchestrator-continue rule (keyed on `wisteria-tag`) so the researcher's
// resumed call hits the researcher rule first.
const KEYWORD_RULES = [
  // Researcher's resumed turn — the harness reconstructs history and appends
  // "[User's answer to your clarification question]\n<message>" as a user
  // message (continue_subagent.rs). Key on the framing so this rule only
  // matches the researcher, not the orchestrator's own continue turn (which
  // sees the composer message without framing).
  { keyword: "user's answer to your clarification question", content: RESEARCHER_FINAL_REPLY },
  // Orchestrator's final synthesis — probe is the tool result carrying the
  // researcher's resumed output. `zenith-beacon` is unique to that reply.
  { keyword: 'zenith-beacon', content: `Done. The result is: ${CANARY_FINAL}` },
  // Orchestrator's continue_subagent turn — user answered the clarification
  // in the composer; the latest user message is USER_ANSWER. The mock
  // substitutes `{{DYNAMIC_TASK_ID}}` / `{{DYNAMIC_AGENT_ID}}` from the
  // preceding `[SUBAGENT_AWAITING_USER]` envelope in message history so the
  // resulting tool_call carries the real runtime-generated task_id.
  {
    keyword: 'wisteria-tag',
    content: '',
    toolCalls: [
      {
        id: 'call_continue_1',
        name: 'continue_subagent',
        arguments: JSON.stringify({
          task_id: '{{DYNAMIC_TASK_ID}}',
          agent_id: '{{DYNAMIC_AGENT_ID}}',
          message: USER_ANSWER,
        }),
      },
    ],
  },
  // Orchestrator's relay turn — probe is the `[SUBAGENT_AWAITING_USER]`
  // envelope; `citrine-cliff` is embedded in the clarification question and
  // does not appear in any later probe.
  { keyword: 'citrine-cliff', content: `The researcher needs to know: ${CLARIFICATION_QUESTION}` },
  // Researcher's initial turn — dispatch renders "Task:\n<DELEGATE_PROMPT>"
  // (archetype_delegation.rs render_structured_handoff) so `harlequin-token`
  // is unique to the researcher's incoming user message.
  {
    keyword: 'harlequin-token',
    content: '',
    toolCalls: [
      {
        id: 'call_clarify_1',
        name: 'ask_user_clarification',
        arguments: JSON.stringify({ question: CLARIFICATION_QUESTION }),
      },
    ],
  },
  // Orchestrator's initial turn — user PROMPT. Shared with the fire-and-forget
  // title-gen call (threadSlice.ts, tools: None) which sees the same probe;
  // `chat_with_system` consumes `content` and ignores unexpected tool_calls,
  // so a delegation-triggering rule here is safe for both callers (benign
  // "Delegating to researcher." title).
  {
    keyword: 'chromatophore-request',
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

describe('Chat harness — orchestrator → subagent continuation flow', () => {
  before(async function beforeSuite() {
    this.timeout(120_000);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);

    setMockBehavior('llmKeywordRules', JSON.stringify(KEYWORD_RULES));
    setMockBehavior('llmStreamChunkDelayMs', '10');
  });

  after(async () => {
    setMockBehavior('llmKeywordRules', '');
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
    // Orchestrator turn 1 + researcher turn 1 + orchestrator relay
    // + orchestrator continue + researcher resumed + orchestrator final
    // = 6, but accept ≥4 for robustness against harness optimisations.
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
