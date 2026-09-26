/**
 * The `todo` tool's whole-list write, as it reaches the DOM — `elements/todo-list`
 * family, through both of its real product paths.
 *
 * There are two renders of the same vendored element and they are not the same
 * claim:
 *
 *   PINNED   `Conversations.tsx:1658` — `data-testid="todo-checklist"`, fed by
 *            `useThreadTodos` off the live `thread_todos_changed` event. One
 *            per thread, always current.
 *   SNAPSHOT `aui/TodoListPart.tsx:76` — one per `todo` tool call in the
 *            transcript, fed by that call's own `args`/`result`
 *            (`TodoListPart.tsx:62-66` is explicit that these are different
 *            sources).
 *
 * `app/test/e2e/specs/chat-todos-goals.spec.ts` (WDIO) owns the pinned list's
 * content across turns, so this spec does not re-assert that. What nothing
 * covers is (a) the transcript snapshots existing at all, (b) an earlier
 * snapshot staying frozen when a later write lands — the property that
 * separates a per-call snapshot from a shared live store — and (c) the
 * single-`in_progress` invariant surviving to the DOM.
 *
 * On (c): the core rejects a list with two `in_progress` items
 * (`crates/openhuman-core/src/agent/tools/todo_tests.rs:77`,
 * `two_in_progress_items_are_rejected`). That is enforced server-side and
 * covered there. What is NOT covered anywhere is what the browser shows when
 * the rejection happens, and the pinned list is the surface that must not lie:
 * it is the user's view of stored state, so it may never show two active
 * items for a write the core refused to store.
 *
 * Status vocabularies differ between the two ends and the mapping is the
 * adapter's job (`TodoListPart.tsx:20-31`): the core writes
 * `pending|in_progress|completed`; the element renders
 * `pending|active|done|failed` and exposes each item's state as a screen-reader
 * span (`todo-list.tsx:82`). Those spans are what this spec reads — real
 * accessibility-tree output, not a test-only attribute.
 *
 * Turns go over `openhuman.channel_web_chat` (`helpers/chat-drive.ts`), never
 * the composer.
 */
import { expect, type Locator, type Page, test } from '@playwright/test';

import {
  resetMock,
  sendTurn,
  setKeywordRules,
  setMockBehavior,
  upstreamBodies,
  waitForSelectedThreadId,
} from '../helpers/chat-drive';
import { bootAuthenticatedPage, dismissWalkthroughIfPresent } from '../helpers/core-rpc';

const USER_ID = 'pw-todo-list-render';

const STEPS = [
  'TODOMARK read the changelog',
  'TODOMARK draft the notes',
  'TODOMARK publish the post',
] as const;

/** Core wire shape: the first `completed` are done, the next is in progress. */
function todos(completed: number) {
  return STEPS.map((content, index) => ({
    content,
    status: index < completed ? 'completed' : index === completed ? 'in_progress' : 'pending',
  }));
}

const RULES = [
  { keyword: 'TODOPLAN', toolCalls: [{ name: 'todo', arguments: { todos: todos(0) } }] },
  { keyword: 'TODOADVANCE', toolCalls: [{ name: 'todo', arguments: { todos: todos(2) } }] },
  {
    // Two `in_progress` items — the core must refuse this write.
    keyword: 'TODOINVALID',
    toolCalls: [
      {
        name: 'todo',
        arguments: {
          todos: [
            { content: STEPS[0], status: 'in_progress' },
            { content: STEPS[1], status: 'in_progress' },
            { content: STEPS[2], status: 'pending' },
          ],
        },
      },
    ],
  },
  // No second-leg rules on purpose. Keying one off the tool RESULT is what the
  // plan-review spec does, but it can there because the core fixes that
  // result's wording verbatim. The `todo` result is produced upstream in
  // `tinytools`, and a rule keyed on a substring like `in_progress` would also
  // match the SUCCESSFUL call's result (its payload echoes the statuses), so
  // the two turns would answer each other's script. The mock's default
  // fall-through ends each turn instead, and the specs below wait on signals
  // that do not depend on the model's words.
];

/**
 * Every rendered todo list, pinned and snapshot alike. `data-slot` is the
 * element's own attribute (`todo-list.tsx:46`), so this finds the real
 * component rather than a wrapper that happens to carry a test id.
 */
const allLists = (page: Page): Locator => page.locator('[data-slot="todo-list"]');

/** The pinned, always-current list above the composer. */
const pinnedList = (page: Page): Locator => page.getByTestId('todo-checklist');

/**
 * The per-tool-call snapshots in the transcript. The pinned render is the only
 * one given a test id (`Conversations.tsx:1659`), so excluding it leaves
 * exactly the `TodoListPart` renders.
 */
const snapshotLists = (page: Page): Locator =>
  page.locator('[data-slot="todo-list"]:not([data-testid])');

/**
 * One list's item states, in list order, as the element publishes them to
 * assistive technology (`todo-list.tsx:82`).
 *
 * Reads the DOM directly rather than through a tolerant helper: a helper that
 * shrugged off a missing span would turn "the element stopped reporting its
 * state" into an empty array, and an empty array compares equal to an empty
 * expectation. The statuses are the subject here, so nothing about them may be
 * forgiving.
 */
async function itemStates(list: Locator): Promise<string[]> {
  return list.evaluate(root =>
    Array.from(root.querySelectorAll('li')).map(item => {
      const span = item.querySelector('span.sr-only');
      // Deliberately not `?? ''`: a missing state span is a defect in the
      // thing under test and must not read as a blank status.
      if (span === null) return '<no-state-span>';
      return (span.textContent ?? '').trim();
    })
  );
}

/** One list's item texts, in list order. */
async function itemTexts(list: Locator): Promise<string[]> {
  return list.evaluate(root =>
    Array.from(root.querySelectorAll('li')).map(item => {
      const clone = item.cloneNode(true) as HTMLElement;
      clone.querySelectorAll('span.sr-only').forEach(node => node.remove());
      return (clone.textContent ?? '').replace(/\s+/g, ' ').trim();
    })
  );
}

async function openChat(page: Page): Promise<void> {
  await bootAuthenticatedPage(page, USER_ID, '/chat');
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('chat-message-input')).toBeVisible({ timeout: 30_000 });
}

test.describe.configure({ timeout: 120_000 });

test.describe('Todo list render', () => {
  test.skip(true, 'legacy todo-list transcript cards were replaced by grouped task insights');
  test.beforeEach(async () => {
    await resetMock();
    await setKeywordRules(RULES);
    await setMockBehavior('llmStreamChunkDelayMs', '10');
  });

  test('a todo tool call renders its own snapshot in the transcript', async ({ page }) => {
    await openChat(page);

    // Control: no list of either kind before the agent writes one, so the
    // assertions below are about something that appeared.
    await expect(allLists(page), 'a todo list was on screen before any write').toHaveCount(0);

    const threadId = await waitForSelectedThreadId(page);
    const before = (await upstreamBodies()).length;
    await sendTurn(page, threadId, 'TODOPLAN write the release plan');

    // Staged upstream-first. "No transcript render" has two very different
    // causes — the tool never ran, or it ran and did not render — and the
    // element assertion alone cannot tell them apart. The round trip is the
    // upstream stage: the tool call goes out, its result comes back, and the
    // harness calls the model again. Assert that happened before blaming the
    // renderer.
    await expect
      .poll(async () => (await upstreamBodies()).length, {
        timeout: 60_000,
        message: 'the scripted todo call never completed a tool round trip',
      })
      .toBeGreaterThan(before + 1);

    await expect(
      snapshotLists(page),
      'the todo tool round-tripped but rendered nothing in the transcript'
    ).toHaveCount(1, { timeout: 60_000 });

    const snapshot = snapshotLists(page).first();
    await expect
      .poll(async () => itemStates(snapshot), { timeout: 30_000 })
      .toEqual(['active', 'pending', 'pending']);
    expect(
      await itemTexts(snapshot),
      'the rendered items are not the ones the tool call carried'
    ).toEqual([...STEPS]);
  });

  test('a later write leaves the earlier snapshot frozen', async ({ page }) => {
    await openChat(page);
    const threadId = await waitForSelectedThreadId(page);

    await sendTurn(page, threadId, 'TODOPLAN write the release plan');
    await expect(snapshotLists(page)).toHaveCount(1, { timeout: 60_000 });
    await expect
      .poll(async () => itemStates(snapshotLists(page).first()), { timeout: 30_000 })
      .toEqual(['active', 'pending', 'pending']);

    await sendTurn(page, threadId, 'TODOADVANCE keep going');
    await expect(
      snapshotLists(page),
      'the second todo call did not produce its own transcript render'
    ).toHaveCount(2, { timeout: 60_000 });

    // The new snapshot carries the new write...
    await expect
      .poll(async () => itemStates(snapshotLists(page).nth(1)), { timeout: 30_000 })
      .toEqual(['done', 'done', 'active']);

    // ...and the first one has NOT moved. A shared store behind both renders
    // would rewrite history: the transcript would show the agent having always
    // known the final state, which is exactly what a transcript must not do.
    expect(
      await itemStates(snapshotLists(page).first()),
      'the earlier transcript snapshot was rewritten by a later write'
    ).toEqual(['active', 'pending', 'pending']);
  });

  test('the pinned list never shows two items in progress', async ({ page }) => {
    await openChat(page);
    const threadId = await waitForSelectedThreadId(page);

    // A valid write first, so the pinned list exists and the assertion below
    // is about its contents rather than its absence.
    await sendTurn(page, threadId, 'TODOPLAN write the release plan');
    await expect(pinnedList(page), 'the pinned checklist never appeared').toBeVisible({
      timeout: 60_000,
    });
    await expect
      .poll(async () => itemStates(pinnedList(page)), { timeout: 30_000 })
      .toEqual(['active', 'pending', 'pending']);

    // Now a write the core refuses (`todo_tests.rs:77`).
    //
    // The rejected call has to have actually round-tripped before the pinned
    // list is worth reading — otherwise "still one active" would just mean
    // nothing had happened yet, and the test would pass on an empty window.
    // Counting upstream requests is the signal that does not depend on the
    // model's wording: the tool call goes out, its result comes back, and the
    // harness calls the model again to continue the turn.
    const before = (await upstreamBodies()).length;
    await sendTurn(page, threadId, 'TODOINVALID do two things at once');
    await expect
      .poll(async () => (await upstreamBodies()).length, {
        timeout: 60_000,
        message: 'the rejected write never completed its tool round trip',
      })
      .toBeGreaterThan(before + 1);

    const states = await itemStates(pinnedList(page));
    expect(
      states.filter(state => state === 'active').length,
      `the pinned checklist showed a list the core refused to store: ${JSON.stringify(states)}`
    ).toBeLessThanOrEqual(1);
  });
});
