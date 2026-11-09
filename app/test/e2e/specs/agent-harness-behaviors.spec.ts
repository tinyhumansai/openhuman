/**
 * Agent-harness behaviors — approval gate, subagent clarification, inference
 * phases, and the tool timeline, driven through the real web-chat stack.
 *
 * This spec starts fresh threads, drives the mock LLM with
 * `llmForcedResponses`, and asserts against the live Redux runtime slice and
 * rendered DOM. Subagent clarification continuation is also exercised by the
 * separate `chat-harness-subagent-continue.spec.ts`; that browser flow is
 * currently skipped under #6389 and is not pass evidence.
 *
 * Approval gate notes (verified against the codebase, not the issue text):
 *   - `ApprovalGate` installs by default under the desktop shell and parks
 *     `Prompt`-class external-effect tool calls on interactive chat turns
 *     (`crates/openhuman-core/src/core/runtime/bootstrap.rs` boot path → `register_approval_surface_subscriber`).
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
import { clickText, textExists } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { setMockBehavior, startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-agent-harness-behaviors';

/** A `file_write` tool call. The mock replays this verbatim as a tool_call. */
function writeToolCall(callId: string, path: string, content: string) {
  return {
    content: '',
    toolCalls: [{ id: callId, name: 'file_write', arguments: JSON.stringify({ path, content }) }],
  };
}

/** A collapsed delegation tool call (orchestrator → researcher subagent). */
function researchToolCall(callId: string, prompt: string) {
  return {
    content: '',
    toolCalls: [
      { id: callId, name: 'delegate_to', arguments: JSON.stringify({ agent: 'research', prompt }) },
    ],
  };
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

/** Start a brand-new chat thread and return its id. */
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

  it('keeps the chat when opening a reply link succeeds or the desktop opener fails', async function () {
    this.timeout(90_000);
    setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([{ content: '[EXTERNAL_PR_LINK](https://example.com/repo/pull/1)' }])
    );
    const threadId = await startNewThread();
    await sendPrompt('show the test PR link');
    expect(await waitForAssistantReplyContaining('EXTERNAL_PR_LINK')).toBe(true);
    const appUrl = await browser.getUrl();
    for (const reject of [false, true]) {
      // Keep this entirely offline: replace only the OS-opener IPC and record
      // any browser fallback. Other IPC continues through the existing bridge.
      await browser.execute((shouldReject: boolean) => {
        const win = window as unknown as {
          isTauri?: boolean;
          __TAURI_INTERNALS__?: { invoke: (...args: unknown[]) => Promise<unknown> };
          __linkTest?: { opened: string[]; fallback: number; restore: () => void };
        };
        const originalRuntime = win.isTauri;
        const originalInternals = win.__TAURI_INTERNALS__;
        const originalOpen = window.open;
        const state = {
          opened: [] as string[],
          fallback: 0,
          restore: () => {
            win.isTauri = originalRuntime;
            win.__TAURI_INTERNALS__ = originalInternals;
            window.open = originalOpen;
          },
        };
        win.__linkTest = state;
        win.isTauri = true;
        win.__TAURI_INTERNALS__ = {
          ...originalInternals,
          invoke: async (...args: unknown[]) => {
            if (args[0] === 'plugin:opener|open_url') {
              state.opened.push((args[1] as { url: string }).url);
              if (shouldReject) throw new Error('mock opener unavailable');
              return;
            }
            return originalInternals?.invoke(...args);
          },
        };
        window.open = () => {
          state.fallback++;
          return null;
        };
      }, reject);
      try {
        await clickText('EXTERNAL_PR_LINK');
        await browser.waitUntil(
          async () =>
            await browser.execute(() => {
              const state = (window as unknown as { __linkTest?: { opened: string[] } }).__linkTest;
              return state?.opened.length === 1;
            }),
          { timeout: 5000 }
        );
        expect(
          await browser.execute(() => {
            const state = (
              window as unknown as { __linkTest?: { opened: string[]; fallback: number } }
            ).__linkTest;
            return { opened: state?.opened, fallback: state?.fallback };
          })
        ).toEqual({ opened: ['https://example.com/repo/pull/1'], fallback: 0 });
        expect(await browser.getUrl()).toBe(appUrl);
        expect(await getSelectedThreadId()).toBe(threadId);
        expect(await chatMounted()).toBe(true);
      } finally {
        await browser.execute(() => {
          const win = window as unknown as { __linkTest?: { restore: () => void } };
          win.__linkTest?.restore();
          delete win.__linkTest;
        });
      }
    }
  });

  it('renders the approval card with approve and deny actions', async function () {
    this.timeout(90_000);
    setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([
        writeToolCall('call_write_seed_a', 'harness-approve.txt', 'seed'),
        writeToolCall('call_write_edit_a', 'harness-approve.txt', 'approved content'),
      ])
    );

    await startNewThread();
    await sendPrompt('please write then update the approve file');

    const card = await $('[role="alertdialog"]');
    await card.waitForDisplayed({ timeout: 45_000 });
    const approve = await $('[data-analytics-id="chat-approval-approve-once"]');
    const deny = await $('[data-analytics-id="chat-approval-deny"]');
    expect(await approve.isDisplayed()).toBe(true);
    expect(await deny.isDisplayed()).toBe(true);

    // Clear the pending UI so it does not leak into later specs in this session.
    expect(await clickApprovalButton('chat-approval-deny')).toBe(true);
    await card.waitForDisplayed({ reverse: true, timeout: 15_000 });
  });

  it('surfaces a subagent clarification question and accepts the user reply', async function () {
    this.timeout(120_000);
    // Keep the question, reply affordance, and visible completion in the browser;
    // Rust owns the underlying clarification and continuation semantics.
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
