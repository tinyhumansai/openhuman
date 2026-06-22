/**
 * Agent-harness behaviors — approval gate, subagent clarification, inference
 * phases, and the tool timeline, driven through the real web-chat stack.
 *
 * Pattern source of truth: `chat-harness-subagent.spec.ts` (orchestrator →
 * subagent delegation, Redux polling via `__OPENHUMAN_STORE__`) and
 * `chat-harness-subagent-continue.spec.ts` (clarification continuation). This
 * spec reuses their proven flow: start a fresh thread, send a prompt, drive the
 * mock LLM with `llmForcedResponses`, and assert against the live Redux runtime
 * slice + the rendered DOM.
 *
 * Approval gate notes (verified against the codebase, not the issue text):
 *   - `ApprovalGate` installs by default under the desktop shell and parks
 *     `Prompt`-class external-effect tool calls on interactive chat turns
 *     (`src/core/jsonrpc.rs` boot path → `register_approval_surface_subscriber`).
 *   - Default autonomy is `Supervised` (`config/schema/autonomy.rs:157`), so the
 *     `Write` command class routes through the gate as `Prompt`
 *     (`security/policy/command_checks.rs:163-166`).
 *   - BUT `file_write.external_effect_with_args` only returns `true` for an
 *     **existing** file ("exists = edit → prompt; new = create → free" —
 *     `filesystem/file_write.rs:65-81`). A brand-new path does NOT park. So to
 *     reliably trigger the gate from a self-contained browser test we issue TWO
 *     `file_write` calls to the SAME path: the first creates the file (no park),
 *     the second edits it (parks → approval card). No workspace pre-seeding or
 *     extra RPC is needed.
 *   - The frontend surfaces the parked call via the `approval_request` socket
 *     event → `setPendingApprovalForThread({ threadId, approval })`
 *     (`providers/ChatRuntimeProvider.tsx:838-864`). State lives at
 *     `chatRuntime.pendingApprovalByThread[threadId]` (a Record keyed by thread,
 *     NOT a single `pendingApproval` field — `chatRuntimeSlice.ts:262`). The
 *     `ApprovalRequestCard` renders `role="alertdialog"` with
 *     `data-analytics-id="chat-approval-approve-once" | "chat-approval-deny"`
 *     (`components/chat/ApprovalRequestCard.tsx:62,90,111`).
 */
import { waitForApp } from '../helpers/app-helpers';
import {
  chatMounted,
  clickByTitle,
  clickSend,
  getSelectedThreadId,
  typeIntoComposer,
  waitForAssistantReplyContaining,
  waitForSocketConnected,
} from '../helpers/chat-harness';
import { textExists } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { setMockBehavior, startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-agent-harness-behaviors';

const APPROVE_CANARY = 'HARNESS_APPROVED_FINAL_77';
const DENY_CANARY = 'HARNESS_DENIED_FINAL_78';

/** A `file_write` tool call. The mock replays this verbatim as a tool_call. */
function writeToolCall(callId: string, path: string, content: string) {
  return {
    content: '',
    toolCalls: [{ id: callId, name: 'file_write', arguments: JSON.stringify({ path, content }) }],
  };
}

/** A `research` delegation tool call (orchestrator → researcher subagent). */
function researchToolCall(callId: string, prompt: string) {
  return {
    content: '',
    toolCalls: [{ id: callId, name: 'research', arguments: JSON.stringify({ prompt }) }],
  };
}

interface PendingApprovalSnapshot {
  requestId?: string;
  toolName?: string;
  message?: string;
}

/** Read `chatRuntime.pendingApprovalByThread[threadId]` from the live store. */
async function readPendingApproval(threadId: string): Promise<PendingApprovalSnapshot | null> {
  return (await browser.execute((tid: string) => {
    const winAny = window as unknown as { __OPENHUMAN_STORE__?: { getState: () => unknown } };
    const state = winAny.__OPENHUMAN_STORE__?.getState() as
      | {
          chatRuntime?: {
            pendingApprovalByThread?: Record<
              string,
              { requestId?: string; toolName?: string; message?: string }
            >;
          };
        }
      | undefined;
    return state?.chatRuntime?.pendingApprovalByThread?.[tid] ?? null;
  }, threadId)) as PendingApprovalSnapshot | null;
}

/** Click an approval-card button by its `data-analytics-id`. Returns whether a
 *  matching, enabled button was found and clicked. */
async function clickApprovalButton(analyticsId: string): Promise<boolean> {
  return (await browser.execute((id: string) => {
    const btn = document.querySelector(`[data-analytics-id="${id}"]`) as HTMLButtonElement | null;
    if (!btn || btn.disabled) return false;
    btn.click();
    return true;
  }, analyticsId)) as boolean;
}

/** Read the current inference phase for a thread, or `'idle'` when the entry
 *  has been removed (idle = entry deleted — `chatRuntimeSlice.ts:440-442`). */
async function readPhase(threadId: string): Promise<string> {
  return (await browser.execute((tid: string) => {
    const winAny = window as unknown as { __OPENHUMAN_STORE__?: { getState: () => unknown } };
    const state = winAny.__OPENHUMAN_STORE__?.getState() as
      | { chatRuntime?: { inferenceStatusByThread?: Record<string, { phase?: string }> } }
      | undefined;
    return state?.chatRuntime?.inferenceStatusByThread?.[tid]?.phase ?? 'idle';
  }, threadId)) as string;
}

/** Whether the thread has any live inference-status entry. */
async function hasInferenceStatus(threadId: string): Promise<boolean> {
  return (await browser.execute((tid: string) => {
    const winAny = window as unknown as { __OPENHUMAN_STORE__?: { getState: () => unknown } };
    const state = winAny.__OPENHUMAN_STORE__?.getState() as
      | { chatRuntime?: { inferenceStatusByThread?: Record<string, unknown> } }
      | undefined;
    return state?.chatRuntime?.inferenceStatusByThread?.[tid] != null;
  }, threadId)) as boolean;
}

interface TimelineEntry {
  id?: string;
  name?: string;
  status?: string;
  round?: number;
}

/** Read `chatRuntime.toolTimelineByThread[threadId]`. */
async function readTimeline(threadId: string): Promise<TimelineEntry[]> {
  return (await browser.execute((tid: string) => {
    const winAny = window as unknown as { __OPENHUMAN_STORE__?: { getState: () => unknown } };
    const state = winAny.__OPENHUMAN_STORE__?.getState() as
      | {
          chatRuntime?: {
            toolTimelineByThread?: Record<
              string,
              Array<{ id?: string; name?: string; status?: string; round?: number }>
            >;
          };
        }
      | undefined;
    return state?.chatRuntime?.toolTimelineByThread?.[tid] ?? [];
  }, threadId)) as TimelineEntry[];
}

/** Find the specific `file_write` timeline entry by its tool-call id. The
 *  approval tests emit two `file_write` calls: a first seed-create that runs
 *  ungated (terminal `success`) and a second *edit* that is parked on the
 *  approval gate. We must assert on the parked edit, so match its exact call id
 *  (the timeline entry `id` is the `tool_call_id`, `ChatRuntimeProvider.tsx:375`).
 *  Matching the seed entry would make the deny case break on the seed's
 *  `success`. Returns `undefined` until the edit entry exists. */
async function findFileWriteEntry(
  threadId: string,
  callId: string
): Promise<TimelineEntry | undefined> {
  const timeline = await readTimeline(threadId);
  return timeline.find(e => e.id === callId);
}

/** Poll until the parked-edit `file_write` entry (`callId`) reaches a terminal
 *  status ('success' | 'error'), then report whether it equals `expected`. We
 *  poll the real execution outcome (`onToolResult` → `event.success ? 'success'
 *  : 'error'`, ChatRuntimeProvider.tsx:420) rather than the replayed canary, so
 *  approve vs deny are genuinely distinguished. On miss the caller dumps the
 *  timeline. */
async function waitForFileWriteStatus(
  threadId: string,
  callId: string,
  expected: 'success' | 'error',
  timeoutMs: number
): Promise<{ ok: boolean; timeline: TimelineEntry[] }> {
  const deadline = Date.now() + timeoutMs;
  let entry: TimelineEntry | undefined;
  while (Date.now() < deadline) {
    entry = await findFileWriteEntry(threadId, callId);
    if (entry && (entry.status === 'success' || entry.status === 'error')) break;
    await browser.pause(100);
  }
  return { ok: entry?.status === expected, timeline: await readTimeline(threadId) };
}

/** Start a brand-new chat thread and return its id. Mirrors the new-thread
 *  flow proven in `chat-harness-subagent.spec.ts`. */
async function startNewThread(): Promise<string> {
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
  return threadId;
}

/** Type the prompt, wait for the socket, and click Send (polling until the
 *  button enables). Shared by every test. */
async function sendPrompt(prompt: string): Promise<void> {
  await typeIntoComposer(prompt);
  const socketReady = await waitForSocketConnected(30_000);
  if (!socketReady) {
    console.warn('[agent-harness-behaviors] socket did not connect within 30 s — send may fail');
  }
  expect(
    await browser.waitUntil(async () => await clickSend(), {
      timeout: 5_000,
      timeoutMsg: 'Send button never enabled',
    })
  ).toBe(true);
}

describe('agent harness behaviors', () => {
  before(async function beforeSuite() {
    this.timeout(120_000);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
    // Faster non-tool streaming so this suite doesn't burn 30s per response.
    setMockBehavior('llmStreamChunkDelayMs', '10');
    await navigateViaHash('/chat');
    await waitForSocketConnected();
  });

  after(async () => {
    setMockBehavior('llmForcedResponses', '');
    setMockBehavior('llmStreamChunkDelayMs', '');
    await stopMockServer();
  });

  it('shows the approval card and completes after the user approves', async function () {
    this.timeout(90_000);
    // First write creates the file (new path → no park); the second write to the
    // SAME path is an edit (exists → parks on the gate). See header note.
    setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([
        writeToolCall('call_write_seed_a', 'harness-approve.txt', 'seed'),
        writeToolCall('call_write_edit_a', 'harness-approve.txt', 'approved content'),
        { content: `Done. ${APPROVE_CANARY}` },
      ])
    );

    const threadId = await startNewThread();
    await sendPrompt('please write then update the approve file');

    // Approval card appears in Redux AND the DOM.
    await browser.waitUntil(async () => (await readPendingApproval(threadId)) !== null, {
      timeout: 45_000,
      timeoutMsg: 'pendingApproval never reached Redux',
    });
    const pending = await readPendingApproval(threadId);
    expect(pending?.toolName).toBe('file_write');
    const card = await $('[role="alertdialog"]');
    await card.waitForDisplayed({ timeout: 10_000 });

    expect(await clickApprovalButton('chat-approval-approve-once')).toBe(true);

    // Final synthesis lands and the parked approval clears.
    const got = await waitForAssistantReplyContaining(APPROVE_CANARY, { timeoutMs: 45_000 });
    expect(got).toBe(true);
    await browser.waitUntil(async () => (await readPendingApproval(threadId)) === null, {
      timeout: 15_000,
      timeoutMsg: 'pendingApproval never cleared after approve',
    });

    // The canary alone is vacuous (it replays regardless of execution), so prove
    // the gated tool ACTUALLY RAN: after approve, file_write executes and its
    // timeline entry settles to 'success' (`onToolResult` sets
    // `event.success ? 'success' : 'error'` — ChatRuntimeProvider.tsx:420).
    const approved = await waitForFileWriteStatus(threadId, 'call_write_edit_a', 'success', 15_000);
    if (!approved.ok) {
      throw new Error(
        `file_write timeline entry never reached 'success' after approve. ` +
          `Timeline: ${JSON.stringify(approved.timeline)}`
      );
    }
  });

  it('denies the tool and the agent acknowledges gracefully', async function () {
    this.timeout(90_000);
    setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([
        writeToolCall('call_write_seed_d', 'harness-deny.txt', 'seed'),
        writeToolCall('call_write_edit_d', 'harness-deny.txt', 'denied content'),
        { content: `Understood, write denied. ${DENY_CANARY}` },
      ])
    );

    const threadId = await startNewThread();
    await sendPrompt('please write then update the deny file');

    await browser.waitUntil(async () => (await readPendingApproval(threadId)) !== null, {
      timeout: 45_000,
      timeoutMsg: 'pendingApproval never reached Redux',
    });
    expect(await clickApprovalButton('chat-approval-deny')).toBe(true);

    const got = await waitForAssistantReplyContaining(DENY_CANARY, { timeoutMs: 45_000 });
    expect(got).toBe(true);
    await browser.waitUntil(async () => (await readPendingApproval(threadId)) === null, {
      timeout: 15_000,
      timeoutMsg: 'pendingApproval never cleared after deny',
    });

    // Prove the gate actually BLOCKED execution (the canary replays either way):
    // on deny the tool does NOT run, the agent loop emits a failed tool
    // completion (`event.success === false`) and the file_write timeline entry
    // settles to 'error' — NOT 'success' (ChatRuntimeProvider.tsx:420). This is
    // the assertion that makes deny genuinely distinct from approve.
    const denied = await waitForFileWriteStatus(threadId, 'call_write_edit_d', 'error', 15_000);
    if (!denied.ok) {
      throw new Error(
        `file_write timeline entry did not settle to 'error' after deny ` +
          `(tool must not have executed). Timeline: ${JSON.stringify(denied.timeline)}`
      );
    }
  });

  it('surfaces a subagent clarification question and accepts the user reply', async function () {
    this.timeout(120_000);
    // NOTE: the full pause/resume `continue_subagent` cycle is covered by
    // `chat-harness-subagent-continue.spec.ts` (and the Rust
    // `subagent_clarification_flow` test). The real `task_id` is dynamic and the
    // mock replays forced responses verbatim (no templating — see
    // `scripts/mock-api/routes/llm.mjs:622-639`), so this browser test verifies
    // the user-visible contract only: the clarification question is shown, the
    // user can reply, and the next turn completes.
    setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([
        // Orchestrator delegates to the researcher.
        researchToolCall('call_research_q', 'need details'),
        // Researcher asks a clarification (early-exits the subagent run).
        {
          content: '',
          toolCalls: [
            {
              id: 'call_clarify_q',
              name: 'ask_user_clarification',
              arguments: JSON.stringify({ question: 'WHICH_FLAVOR_CANARY?' }),
            },
          ],
        },
        // Orchestrator relays the question to the user (turn ends, input needed).
        { content: 'Quick question: WHICH_FLAVOR_CANARY?' },
        // User replies → next turn answers with the final canary.
        { content: 'Great, going with chocolate. FLAVOR_FINAL_CANARY' },
      ])
    );

    await startNewThread();
    await sendPrompt('run the flavor research');

    // Intermediate clarification question is visible in chat.
    await browser.waitUntil(async () => await textExists('WHICH_FLAVOR_CANARY'), {
      timeout: 60_000,
      timeoutMsg: 'clarification question never shown',
    });

    // User replies and the flow completes.
    await sendPrompt('chocolate');
    const got = await waitForAssistantReplyContaining('FLAVOR_FINAL_CANARY', { timeoutMs: 60_000 });
    expect(got).toBe(true);
  });

  it('transitions through subagent inference phases and clears to idle', async function () {
    this.timeout(120_000);
    setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([
        researchToolCall('call_research_p', 'phase check'),
        { content: 'PHASE_SUB_ANSWER' },
        { content: 'All phases done. PHASE_FINAL_CANARY' },
      ])
    );

    const threadId = await startNewThread();
    await sendPrompt('check the phases');

    // Collect observed phases until the final canary lands.
    const seen = new Set<string>();
    const deadline = Date.now() + 60_000;
    while (Date.now() < deadline) {
      seen.add(await readPhase(threadId));
      if (await textExists('PHASE_FINAL_CANARY')) break;
      // Sample at 50ms: with the mock's 10ms stream delay plus real LLM
      // round-trips the subagent phase window spans hundreds of ms, so 50ms
      // polling reliably catches it (150ms could miss the transient window).
      await browser.pause(50);
    }

    // Real phase values: 'thinking' | 'tool_use' | 'subagent'; idle = no entry.
    expect(seen.has('subagent')).toBe(true);
    expect(seen.has('thinking') || seen.has('tool_use')).toBe(true);

    // Status clears (entry removed) once the turn finishes.
    await browser.waitUntil(async () => !(await hasInferenceStatus(threadId)), {
      timeout: 30_000,
      timeoutMsg: 'inference status never cleared to idle',
    });
  });

  it('records a complete tool timeline for a subagent turn', async function () {
    this.timeout(120_000);
    setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([
        researchToolCall('call_research_t', 'timeline check'),
        { content: 'TIMELINE_SUB_ANSWER' },
        { content: 'Timeline complete. TIMELINE_FINAL_CANARY' },
      ])
    );

    const threadId = await startNewThread();
    await sendPrompt('check the timeline');
    const got = await waitForAssistantReplyContaining('TIMELINE_FINAL_CANARY', {
      timeoutMs: 60_000,
    });
    expect(got).toBe(true);

    const timeline = await readTimeline(threadId);
    expect(timeline.length).toBeGreaterThan(0);
    for (const entry of timeline) {
      expect(typeof entry.id).toBe('string');
      expect((entry.id ?? '').length).toBeGreaterThan(0);
      expect(typeof entry.name).toBe('string');
      expect(['running', 'success', 'error', 'awaiting_user']).toContain(entry.status ?? '');
      expect(typeof entry.round).toBe('number');
    }

    // A subagent entry is present and finished successfully.
    const sub = timeline.find(
      e => (e.id ?? '').includes(':subagent:') || (e.name ?? '').startsWith('subagent:')
    );
    expect(sub).toBeDefined();
    expect(sub?.status).toBe('success');

    // Rounds are monotonically non-decreasing (timeline ordering).
    const rounds = timeline.map(e => e.round ?? 0);
    expect([...rounds].sort((a, b) => a - b)).toEqual(rounds);
  });
});
