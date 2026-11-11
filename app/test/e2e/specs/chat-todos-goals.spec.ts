// @ts-nocheck
/**
 * Harness work state in the chat pane — the todo checklist and the goal
 * banner, driven end to end by a real agent turn.
 *
 * The agent owns both: it writes the whole todo list with one `todo` call
 * and records the thread's objective with `goal_set`. Neither has an RPC;
 * the pane reads the newest tool result off the timeline
 * (`app/src/features/conversations/utils/harnessState.ts`). So this spec is
 * the proof that what the core answers actually reaches the two surfaces a
 * user sees.
 *
 * The scripted turn:
 *   Turn 1: goal_set → todo (five items, first in progress) → text
 *   Turn 2: todo (three completed, fourth in progress) → text
 *   Turn 3: todo (all five completed) → goal_complete
 *
 * Verifies:
 *   G1.1 — after the scripted turn settles, the task card shows the objective
 *          and waiting status
 *   G1.2 — the checklist renders all five items with their statuses
 *   G1.3 — a later write moves the checklist on (3 of 5, fourth in progress)
 *   G1.4 — the final write completes every item and the banner turns complete
 *   G1.5 — both survive a thread switch and switch back, rebuilt from the
 *          thread's persisted turn states (`useThreadHarnessState`)
 */
import { waitForApp } from '../helpers/app-helpers';
import {
  chatMounted,
  clickByTitle,
  clickSend,
  getSelectedThreadId,
  typeIntoComposer,
  waitForSocketConnected,
} from '../helpers/chat-harness';
import { clickSelector, clickTestId, waitForTestId } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { setMockBehavior, startMockServer, stopMockServer } from '../mock-server';

const LOG_PREFIX = '[chat-todos-goals]';
const USER_ID = 'e2e-chat-todos-goals';
const OBJECTIVE = 'Ship the v2 release notes';

const STEPS = [
  'Read the changelog',
  'Draft the notes',
  'Check the version numbers',
  'Get review sign-off',
  'Publish the post',
];

/** The `todo` arguments for "the first `completed` items are done". */
function todoArgs(completed: number): string {
  return JSON.stringify({
    todos: STEPS.map((content, i) => ({
      content,
      status: i < completed ? 'completed' : i === completed ? 'in_progress' : 'pending',
    })),
  });
}

function toolCall(id: string, name: string, args: string) {
  return { content: '', toolCalls: [{ id, name, arguments: args }] };
}

const TURN_1_RESPONSES = [
  // One agent turn may make several primary model requests as tools run.
  // Keep this queue scoped to the first user message so later turns retain
  // their own observable intermediate state.
  toolCall(
    'call_goal_set_1',
    'goal_set',
    JSON.stringify({ objective: OBJECTIVE, token_budget: 50000 })
  ),
  toolCall('call_todo_1', 'todo', todoArgs(0)),
  { content: 'Plan written; starting on the changelog.' },
];

const TURN_2_RESPONSES = [
  // Turn 2: three done, working the fourth.
  toolCall('call_todo_2', 'todo', todoArgs(3)),
  { content: 'Three steps done.' },
];

const TURN_3_RESPONSES = [
  // Turn 3: everything done, goal closed.
  toolCall('call_todo_3', 'todo', todoArgs(5)),
  // goal_complete is terminal by the TinyAgents contract; it suppresses
  // autonomous continuation, so there is no subsequent model text response.
  toolCall('call_goal_complete_1', 'goal_complete', JSON.stringify({})),
];

/** The checklist's rendered items: `[content, status]` pairs, in list order. */
async function readChecklist(): Promise<Array<[string, string]>> {
  return (await browser.execute(() => {
    const card = document.querySelector('[data-testid="todo-checklist"]');
    const rows = Array.from(card?.querySelectorAll('[data-testid="todo-item"]') ?? []);
    return rows.map(row => [(row.textContent ?? '').trim(), row.getAttribute('data-status') ?? '']);
  })) as Array<[string, string]>;
}

/** The current task-card checklist's completed/total item counts. */
async function readProgress(): Promise<{ completed: number; total: number } | null> {
  const items = await readChecklist();
  if (!items.length) return null;
  return { completed: items.filter(([, status]) => status === 'done').length, total: items.length };
}

/** The shipped task card's state + objective, or null when no card is shown. */
async function readGoal(): Promise<{ status: string; objective: string } | null> {
  return (await browser.execute(() => {
    const el = document.querySelector('[data-testid="todo-checklist"]');
    if (!el) return null;
    // TaskCard puts the objective in its flexing label span. Whole-button
    // text also includes the screen-reader state and the trailing meta label.
    const label = el.querySelector('button > span.flex-1');
    return {
      status: el.getAttribute('data-state') ?? '',
      objective: (label?.textContent ?? '').trim(),
    };
  })) as { status: string; objective: string } | null;
}

/** Content-free identity check for the thread that owns the visible turn. */
async function readSelectedThreadSnapshot(): Promise<{
  threadId: string | null;
  messageCount: number;
  hasUserMessage: boolean;
  hasAgentMessage: boolean;
}> {
  return (await browser.execute(() => {
    const winAny = window as unknown as { __OPENHUMAN_STORE__?: { getState: () => unknown } };
    const state = winAny.__OPENHUMAN_STORE__?.getState() as
      | {
          thread?: {
            selectedThreadId?: string | null;
            messagesByThreadId?: Record<string, Array<{ sender?: 'user' | 'agent' }>>;
          };
        }
      | undefined;
    const threadId = state?.thread?.selectedThreadId ?? null;
    const messages = threadId ? (state?.thread?.messagesByThreadId?.[threadId] ?? []) : [];
    return {
      threadId,
      messageCount: messages.length,
      hasUserMessage: messages.some(message => message.sender === 'user'),
      hasAgentMessage: messages.some(message => message.sender === 'agent'),
    };
  })) as {
    threadId: string | null;
    messageCount: number;
    hasUserMessage: boolean;
    hasAgentMessage: boolean;
  };
}

async function waitForProgress(completed: number, total: number, timeout = 45_000) {
  await browser.waitUntil(
    async () => {
      const progress = await readProgress();
      return progress?.completed === completed && progress?.total === total;
    },
    {
      timeout,
      timeoutMsg: `checklist never reached ${completed}/${total}; last: ${JSON.stringify(
        await readProgress()
      )}`,
    }
  );
}

async function waitForGoalStatus(status: string, timeout = 45_000): Promise<void> {
  await browser.waitUntil(async () => (await readGoal())?.status === status, {
    timeout,
    timeoutMsg: `task card never reached ${status}; last: ${JSON.stringify(await readGoal())}`,
  });
}

/** Completed task cards start collapsed, so expand before inspecting their rows. */
async function expandChecklistIfCollapsed(): Promise<void> {
  const expanded = (await browser.execute(() =>
    document.querySelector('[data-testid="todo-checklist"] button')?.getAttribute('aria-expanded')
  )) as string | null | undefined;
  if (expanded === 'false') {
    await clickSelector('[data-testid="todo-checklist"] button');
  }
  await browser.waitUntil(
    async () =>
      (await browser.execute(() =>
        document
          .querySelector('[data-testid="todo-checklist"] button')
          ?.getAttribute('aria-expanded')
      )) === 'true',
    { timeout: 5_000, timeoutMsg: 'task checklist disclosure did not open' }
  );
}

async function sendTurn(message: string) {
  await typeIntoComposer(message);
  expect(
    await browser.waitUntil(async () => await clickSend(), {
      timeout: 5_000,
      timeoutMsg: 'Send button never enabled',
    })
  ).toBe(true);
}

describe('Chat todos and goals', () => {
  let threadId: string;

  before(async () => {
    console.log(`${LOG_PREFIX} Starting mock server and resetting app`);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);

    setMockBehavior('llmStreamChunkDelayMs', '10');
    console.log(`${LOG_PREFIX} Setup complete — per-turn forced responses ready`);
  });

  after(async () => {
    setMockBehavior('llmForcedResponses', '');
    setMockBehavior('llmStreamChunkDelayMs', '');
    await stopMockServer();
  });

  it('G1.1 — the goal banner shows the objective the agent set', async () => {
    await navigateViaHash('/chat');
    await browser.waitUntil(async () => await chatMounted(), {
      timeout: 15_000,
      timeoutMsg: 'Conversations panel did not mount',
    });
    expect(await clickByTitle('New thread', 8_000)).toBe(true);

    threadId = (await browser.waitUntil(async () => await getSelectedThreadId(), {
      timeout: 8_000,
      timeoutMsg: 'thread.selectedThreadId never populated',
    })) as string;
    console.log(`${LOG_PREFIX} thread: ${threadId}`);

    const socketReady = await waitForSocketConnected(30_000);
    if (!socketReady) console.warn(`${LOG_PREFIX} socket not connected — send may fail`);
    setMockBehavior('llmForcedResponses', JSON.stringify(TURN_1_RESPONSES));
    await sendTurn('Write the v2 release notes, in five steps.');

    await waitForTestId('todo-checklist', 45_000);
    await waitForGoalStatus('waiting');
    const goal = await readGoal();
    console.log(`${LOG_PREFIX} G1.1: task card — ${JSON.stringify(goal)}`);
    expect(goal?.objective).toBe(OBJECTIVE);
    expect(goal?.status).toBe('waiting');

    // New-thread creation can expose its placeholder selection before the
    // durable turn is attached. Track the actual selected thread only after
    // Redux proves it owns this rendered conversation.
    const placeholderThreadId = threadId;
    await browser.waitUntil(
      async () => {
        const current = await readSelectedThreadSnapshot();
        return Boolean(current.threadId && current.hasUserMessage && current.hasAgentMessage);
      },
      {
        timeout: 15_000,
        timeoutMsg: `selected thread never owned the rendered turn: ${JSON.stringify(
          await readSelectedThreadSnapshot()
        )}`,
      }
    );
    const current = await readSelectedThreadSnapshot();
    expect(current.threadId).toBeTruthy();
    expect(current.hasUserMessage).toBe(true);
    expect(current.hasAgentMessage).toBe(true);
    console.log(
      `${LOG_PREFIX} G1.1 identity: placeholder_changed=${current.threadId !== placeholderThreadId}; selected_message_count=${current.messageCount}`
    );
    threadId = current.threadId!;
  });

  it('G1.2 — the checklist renders all five items with their statuses', async () => {
    await waitForTestId('todo-checklist', 45_000);
    await waitForProgress(0, 5);
    await waitForGoalStatus('waiting');

    const items = await readChecklist();
    console.log(`${LOG_PREFIX} G1.2: items — ${JSON.stringify(items)}`);
    expect(items.map(([content]) => content.replace(/\s+/g, ' ').trim())).toEqual(
      STEPS.map(step => expect.stringContaining(step))
    );
    expect(items.map(([, status]) => status)).toEqual([
      'pending',
      'pending',
      'pending',
      'pending',
      'pending',
    ]);
  });

  it('G1.3 — a later write moves the checklist on', async () => {
    setMockBehavior('llmForcedResponses', JSON.stringify(TURN_2_RESPONSES));
    await sendTurn('continue');
    await waitForProgress(3, 5);
    await waitForGoalStatus('waiting');

    const items = await readChecklist();
    console.log(`${LOG_PREFIX} G1.3: items — ${JSON.stringify(items)}`);
    expect(items.map(([, status]) => status)).toEqual([
      'done',
      'done',
      'done',
      'pending',
      'pending',
    ]);
    // The goal is untouched by a todo write.
    expect(['working', 'waiting']).toContain((await readGoal())?.status);
  });

  it('G1.4 — the final turn completes every item and marks the task card done', async () => {
    setMockBehavior('llmForcedResponses', JSON.stringify(TURN_3_RESPONSES));
    await sendTurn('finish it');
    await browser.waitUntil(async () => (await readGoal())?.status === 'done', {
      timeout: 45_000,
      timeoutMsg: `goal never turned complete; last: ${JSON.stringify(await readGoal())}`,
    });
    await expandChecklistIfCollapsed();
    await waitForProgress(5, 5);

    const items = await readChecklist();
    expect(items.map(([, status]) => status)).toEqual(['done', 'done', 'done', 'done', 'done']);
    expect(await readGoal()).toMatchObject({ status: 'done', objective: OBJECTIVE });
    const current = await readSelectedThreadSnapshot();
    expect(current.threadId).toBe(threadId);
    expect(current.hasUserMessage).toBe(true);
    expect(current.hasAgentMessage).toBe(true);
    console.log(`${LOG_PREFIX} G1.4: passed — all five done, goal complete`);
  });

  it('G1.5 — both survive a thread switch and back', async () => {
    // A second thread has neither surface: the state is per-thread.
    expect(await clickByTitle('New thread', 8_000)).toBe(true);
    await browser.waitUntil(
      async () => {
        const id = await getSelectedThreadId();
        return typeof id === 'string' && id !== threadId;
      },
      { timeout: 8_000, timeoutMsg: 'second thread never became selected' }
    );
    await browser.waitUntil(async () => (await readProgress()) === null, {
      timeout: 15_000,
      timeoutMsg: 'checklist leaked into a different thread',
    });
    expect(await readGoal()).toBeNull();

    // Back to the first thread: both rebuild from the thread's persisted turn
    // states — the goal from the turn that set it, the list from the last
    // write.
    await clickTestId(`thread-row-${threadId}`, 15_000);
    await browser.waitUntil(async () => (await getSelectedThreadId()) === threadId, {
      timeout: 8_000,
      timeoutMsg: 'never switched back to the first thread',
    });

    await browser.waitUntil(async () => (await readGoal())?.status === 'done', {
      timeout: 30_000,
      timeoutMsg: `goal did not rehydrate; last: ${JSON.stringify(await readGoal())}`,
    });
    await expandChecklistIfCollapsed();
    await waitForProgress(5, 5);
    expect((await readGoal())?.objective).toBe(OBJECTIVE);
    console.log(`${LOG_PREFIX} G1.5: passed — both rehydrated after a thread switch`);
  });
});
