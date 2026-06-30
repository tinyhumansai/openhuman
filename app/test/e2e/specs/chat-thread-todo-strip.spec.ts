// @ts-nocheck
/**
 * Chat thread todo strip — end-to-end.
 *
 * Exercises the per-conversation-thread todo list, top to bottom:
 *   - The mock LLM emits a `todo` tool call (op:add) on turn 1, then a final
 *     answer on turn 2.
 *   - The Rust core executes the thread-bound `todo` tool against the selected
 *     thread's task board and emits a `task_board_updated` progress event.
 *   - The frontend records the board into `chatRuntime.taskBoardByThread[tid]`.
 *   - The read-only `ThreadTodoStrip` (`[data-testid="thread-todo-strip"]`)
 *     renders above the composer with the agent-authored card.
 *
 * This is the feature E2E for the "todo list per thread" surface: it proves the
 * agent can author its plan AND that the plan renders in the chat view — the
 * full path the unit tests can only stub.
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
import { textExists } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { clearRequestLog, setMockBehavior, startMockServer, stopMockServer } from '../mock-server';

const LOG_PREFIX = '[chat-thread-todo-strip]';
const USER_ID = 'e2e-chat-thread-todo-strip';
const PROMPT = 'Plan the multi-step refactor before you start.';
const CARD_TITLE = 'canary-todo-card-7e2a9d';
const FINAL_REPLY = 'Plan recorded — starting now. canary-final-3f1c';

// Turn 1: the LLM writes a card to the thread board via the `todo` tool.
// Turn 2: it answers now that the plan exists.
const FORCED_RESPONSES = [
  {
    content: '',
    toolCalls: [
      {
        id: 'call_todo_add_1',
        name: 'todo',
        arguments: JSON.stringify({ op: 'add', content: CARD_TITLE, status: 'in_progress' }),
      },
    ],
  },
  { content: FINAL_REPLY },
];

/** Read the selected thread's board cards straight out of the redux store. */
async function boardCardTitles(threadId: string): Promise<string[]> {
  return (await browser.execute((tid: string) => {
    const winAny = window as unknown as { __OPENHUMAN_STORE__?: { getState: () => unknown } };
    const state = winAny.__OPENHUMAN_STORE__?.getState() as
      | {
          chatRuntime?: {
            taskBoardByThread?: Record<string, { cards?: Array<{ title?: string }> }>;
          };
        }
      | undefined;
    const board = state?.chatRuntime?.taskBoardByThread?.[tid];
    return (board?.cards ?? []).map(c => c?.title ?? '');
  }, threadId)) as string[];
}

describe('Chat thread todo strip', () => {
  before(async function beforeSuite() {
    this.timeout(90_000);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);

    setMockBehavior('llmForcedResponses', JSON.stringify(FORCED_RESPONSES));
    setMockBehavior('llmStreamChunkDelayMs', '10');
    clearRequestLog();
    console.log(`${LOG_PREFIX} setup complete — forced todo tool-call configured`);
  });

  after(async () => {
    setMockBehavior('llmForcedResponses', '');
    setMockBehavior('llmStreamChunkDelayMs', '');
    await stopMockServer();
  });

  it('renders the agent-authored card in the thread todo strip above the composer', async () => {
    await navigateViaHash('/chat');
    await browser.waitUntil(async () => await chatMounted(), {
      timeout: 15_000,
      timeoutMsg: 'Conversations panel did not mount',
    });
    expect(await clickByTitle('New thread', 8_000)).toBe(true);

    const threadId = (await browser.waitUntil(async () => await getSelectedThreadId(), {
      timeout: 8_000,
      timeoutMsg: 'thread.selectedThreadId never populated',
    })) as string;
    expect(typeof threadId).toBe('string');

    await typeIntoComposer(PROMPT);
    if (!(await waitForSocketConnected(30_000))) {
      console.warn(`${LOG_PREFIX} socket did not connect within 30 s — send may fail`);
    }
    expect(
      await browser.waitUntil(async () => await clickSend(), {
        timeout: 5_000,
        timeoutMsg: 'Send button never enabled',
      })
    ).toBe(true);

    // The board lands in redux once the core processes the `todo` tool call and
    // emits task_board_updated.
    await browser.waitUntil(async () => (await boardCardTitles(threadId)).includes(CARD_TITLE), {
      timeout: 45_000,
      timeoutMsg: 'thread board never received the agent-authored card',
    });

    // The read-only strip mounts above the composer and lists the active card.
    const strip = await $('[data-testid="thread-todo-strip"]');
    await strip.waitForExist({ timeout: 15_000 });
    expect(await strip.isDisplayed()).toBe(true);
    expect(await textExists(CARD_TITLE)).toBe(true);
    console.log(`${LOG_PREFIX} passed — strip rendered the agent-authored card`);
  });
});
