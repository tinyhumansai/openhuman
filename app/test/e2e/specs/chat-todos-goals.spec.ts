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
 *   Turn 3: todo (all five completed) → goal_complete → text
 *
 * Verifies:
 *   G1.1 — the goal banner shows the objective and an "active" status
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
import { clickTestId, textExists, waitForTestId } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { setMockBehavior, startMockServer, stopMockServer } from '../mock-server';

const LOG_PREFIX = '[chat-todos-goals]';
const USER_ID = 'e2e-chat-todos-goals';
const OBJECTIVE = 'Ship the v2 release notes';
const CANARY_FINAL = 'canary-todos-goals-9a8b7c';

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

const FORCED_RESPONSES = [
  // Turn 1: record the objective, then write the plan.
  toolCall(
    'call_goal_set_1',
    'goal_set',
    JSON.stringify({ objective: OBJECTIVE, token_budget: 50000 })
  ),
  toolCall('call_todo_1', 'todo', todoArgs(0)),
  { content: 'Plan written; starting on the changelog.' },
  // Turn 2: three done, working the fourth.
  toolCall('call_todo_2', 'todo', todoArgs(3)),
  { content: 'Three steps done.' },
  // Turn 3: everything done, goal closed.
  toolCall('call_todo_3', 'todo', todoArgs(5)),
  toolCall('call_goal_complete_1', 'goal_complete', JSON.stringify({})),
  { content: `All five steps are done: ${CANARY_FINAL}` },
];

/** The checklist's rendered items: `[content, status]` pairs, in list order. */
async function readChecklist(): Promise<Array<[string, string]>> {
  return (await browser.execute(() => {
    const rows = Array.from(document.querySelectorAll('[data-testid="todo-item"]'));
    return rows.map(row => [(row.textContent ?? '').trim(), row.getAttribute('data-status') ?? '']);
  })) as Array<[string, string]>;
}

/** The checklist header's completed/total counters, or null when absent. */
async function readProgress(): Promise<{ completed: number; total: number } | null> {
  return (await browser.execute(() => {
    const el = document.querySelector('[data-testid="todo-checklist"]');
    if (!el) return null;
    return {
      completed: Number(el.getAttribute('data-todo-completed') ?? -1),
      total: Number(el.getAttribute('data-todo-total') ?? -1),
    };
  })) as { completed: number; total: number } | null;
}

/** The goal banner's status + objective, or null when no banner is shown. */
async function readGoal(): Promise<{ status: string; objective: string } | null> {
  return (await browser.execute(() => {
    const el = document.querySelector('[data-testid="goal-banner"]');
    if (!el) return null;
    const objective = el.querySelector('[data-testid="goal-objective"]');
    return {
      status: el.getAttribute('data-goal-status') ?? '',
      objective: (objective?.textContent ?? '').trim(),
    };
  })) as { status: string; objective: string } | null;
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
  before(function () {
    this.skip();
  });
  let threadId: string;

  before(async () => {
    console.log(`${LOG_PREFIX} Starting mock server and resetting app`);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);

    setMockBehavior('llmForcedResponses', JSON.stringify(FORCED_RESPONSES));
    setMockBehavior('llmStreamChunkDelayMs', '10');
    console.log(`${LOG_PREFIX} Setup complete — ${FORCED_RESPONSES.length} forced responses`);
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
    await sendTurn('Write the v2 release notes, in five steps.');

    await waitForTestId('goal-banner', 45_000);
    const goal = await readGoal();
    console.log(`${LOG_PREFIX} G1.1: banner — ${JSON.stringify(goal)}`);
    expect(goal?.objective).toBe(OBJECTIVE);
    expect(goal?.status).toBe('active');
  });

  it('G1.2 — the checklist renders all five items with their statuses', async () => {
    await waitForTestId('todo-checklist', 45_000);
    await waitForProgress(0, 5);

    const items = await readChecklist();
    console.log(`${LOG_PREFIX} G1.2: items — ${JSON.stringify(items)}`);
    expect(items.map(([content]) => content.replace(/\s+/g, ' ').trim())).toEqual(
      STEPS.map(step => expect.stringContaining(step))
    );
    expect(items.map(([, status]) => status)).toEqual([
      'in_progress',
      'pending',
      'pending',
      'pending',
      'pending',
    ]);
  });

  it('G1.3 — a later write moves the checklist on', async () => {
    await sendTurn('continue');
    await waitForProgress(3, 5);

    const items = await readChecklist();
    console.log(`${LOG_PREFIX} G1.3: items — ${JSON.stringify(items)}`);
    expect(items.map(([, status]) => status)).toEqual([
      'completed',
      'completed',
      'completed',
      'in_progress',
      'pending',
    ]);
    // The goal is untouched by a todo write.
    expect((await readGoal())?.status).toBe('active');
  });

  it('G1.4 — the final turn completes every item and closes the goal', async () => {
    await sendTurn('finish it');
    await waitForProgress(5, 5);
    await browser.waitUntil(async () => (await readGoal())?.status === 'complete', {
      timeout: 45_000,
      timeoutMsg: `goal never turned complete; last: ${JSON.stringify(await readGoal())}`,
    });

    const items = await readChecklist();
    expect(items.map(([, status]) => status)).toEqual([
      'completed',
      'completed',
      'completed',
      'completed',
      'completed',
    ]);
    expect(await textExists(CANARY_FINAL)).toBe(true);
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

    await waitForProgress(5, 5);
    await browser.waitUntil(async () => (await readGoal())?.status === 'complete', {
      timeout: 30_000,
      timeoutMsg: `goal did not rehydrate; last: ${JSON.stringify(await readGoal())}`,
    });
    expect((await readGoal())?.objective).toBe(OBJECTIVE);
    console.log(`${LOG_PREFIX} G1.5: passed — both rehydrated after a thread switch`);
  });
});
