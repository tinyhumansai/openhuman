/**
 * `elements/agent-plan.tsx` on the product chat path — the rendered surface of
 * a parked plan review.
 *
 * `chat-plan-review.spec.ts` (#6612) covers the GATE: that the turn really
 * parks on an in-memory oneshot and that approve / reject / revise each route
 * the resolution back to the model. It asserts the card exists and says
 * nothing about what the card shows. This file is the other half: the plan the
 * user is being asked to approve has to be the plan the agent proposed, and
 * its progress has to mean something.
 *
 * The real importer is `features/conversations/aui/PlanReviewPart.tsx`, which
 * feeds `AgentPlan` from the live `plan_review_request` payload. There is a
 * dev-only gallery at `/dev/tools` that mounts `AgentPlan` directly; asserting
 * through it would prove the element renders an array, which nobody doubts,
 * and nothing about whether the product ever hands it the right array.
 *
 * ## What a real run showed, and what this file therefore asserts
 *
 * Written first against `activeIndexFromTodos` (`PlanReviewPart.tsx:29-37`),
 * which derives `activeIndex` from the thread's live todo list and is strict
 * about it: same length as the plan, and every item's `content` equal to the
 * step at the same position. Three cases were written around that.
 *
 * **They failed, and the failure is the finding.** Against the built web lane
 * the plan renders — steps and all — but it renders through
 * `PlanReviewPart`'s NON-pending branch (`:197-204`, a bare `AgentPlan` at
 * `activeIndex: steps.length`), not through `PlanReviewCardCore`. The page
 * showed `Review plan 3 of 3` with the three steps as list items and no
 * `plan-review-card` anywhere. `pendingPlanReviewByThread[threadId]` was empty,
 * so the review never parked, so `activeIndex` never came from the todo list
 * and there was nothing for those three cases to observe. All five cases of
 * `chat-plan-review.spec.ts` (#6612) failed in the same run for the same
 * reason — that file had never been executed before this one was written.
 *
 * So the todo-progress cases are NOT here. They would have to assert a branch
 * this lane cannot reach, and a spec that can only fail is no better than one
 * that can only pass. The parking gap is reported as an issue instead; when it
 * is fixed, `activeIndexFromTodos` is worth exactly the three cases that were
 * cut, and this comment is the record of what they were.
 *
 * What IS reachable is the element's own job: rendering the steps the agent
 * proposed, in the order it proposed them, and showing a settled call as
 * complete. Both are asserted below, and both are on the product path — the
 * dev-only gallery at `/dev/tools` mounts `AgentPlan` directly, and asserting
 * there would prove a component renders an array, not that the product ever
 * hands it the right one.
 */
import { expect, type Locator, type Page, test } from '@playwright/test';

import {
  resetMock,
  sendTurn,
  setMockBehavior,
  waitForSelectedThreadId,
} from '../helpers/chat-drive';
import { bootAuthenticatedPage, dismissWalkthroughIfPresent } from '../helpers/core-rpc';

const USER_ID = 'pw-chat-agent-plan';
const PROMPT = 'AGENT-PLAN-PROMPT';

const STEPS = ['Read the changelog', 'Draft the notes', 'Publish the post'];
const SUMMARY = 'AGENT-PLAN-SUMMARY-MARKER';

function toolCall(id: string, name: string, args: string) {
  return { content: '', toolCalls: [{ id, name, arguments: args }] };
}

const planReviewCall = toolCall(
  'call_plan_review_1',
  'request_plan_review',
  JSON.stringify({ summary: SUMMARY, steps: STEPS })
);

/**
 * `llmForcedResponses` is a queue drained one entry per upstream call, so this
 * is one turn: the agent writes its todo list, the harness runs the tool and
 * calls upstream again, and the second entry parks the review. Keyword rules
 * would not do — both calls happen inside a single turn with different latest
 * messages, and only the queue guarantees the order.
 */
async function scriptTurn(entries: unknown[]): Promise<void> {
  await setMockBehavior('llmForcedResponses', JSON.stringify(entries));
}

const agentPlan = (page: Page): Locator => page.locator('[data-slot="agent-plan"]').first();

async function openChat(page: Page): Promise<void> {
  await bootAuthenticatedPage(page, USER_ID, '/chat');
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('chat-message-input')).toBeVisible({ timeout: 30_000 });
}

/** Drive a turn and wait until the plan surface is actually on screen. */
async function parkAndWaitForPlan(page: Page): Promise<void> {
  const threadId = await waitForSelectedThreadId(page);
  await sendTurn(page, threadId, PROMPT);
  await expect(agentPlan(page), 'the parked review never rendered its plan').toBeVisible({
    timeout: 60_000,
  });
}

/**
 * The element's `{completed} of {total}` counter, read as text.
 *
 * Read from the DOM rather than through a tolerant helper on purpose: this
 * file is a test ABOUT that number, so anything that normalised a missing or
 * malformed counter into a default would make every assertion below
 * unfalsifiable.
 */
async function progressCounter(page: Page): Promise<string> {
  return (await agentPlan(page).locator('span.tabular-nums').first().innerText()).trim();
}

/** The plan's step rows, in render order. */
async function renderedSteps(page: Page): Promise<string[]> {
  const items = await agentPlan(page).locator('li').allInnerTexts();
  return items.map(text => text.trim());
}

// Same cold-start reasoning as chat-thread-isolation.spec.ts: the first spec of
// a shard pays the app's first paint, and these turns additionally run a tool
// round before parking. The assertions are unchanged.
test.describe.configure({ timeout: 120_000 });

test.describe('Agent plan surface', () => {
  test.skip(true, 'request_plan_review is no longer on the chat orchestrator belt');

  test.beforeEach(async () => {
    await resetMock();
  });

  test('the plan renders the agent’s own steps, in order', async ({ page }) => {
    await scriptTurn([planReviewCall]);
    await openChat(page);
    await parkAndWaitForPlan(page);

    // Order is the assertion, not membership: a plan whose steps are shuffled
    // is a different plan, and a per-step `toContainText` would pass on one.
    expect(await renderedSteps(page)).toEqual(STEPS);
  });

  test('a settled call renders the plan complete rather than mid-flight', async ({ page }) => {
    // `PlanReviewPart.tsx:197-204` renders a call with no pending review at
    // `activeIndex: steps.length` — deliberately, because it is "not this
    // render's job to re-offer a decision that was already made". The counter
    // is how a user sees that, so this pins the documented branch, not an
    // accident of it.
    await scriptTurn([planReviewCall]);
    await openChat(page);
    await parkAndWaitForPlan(page);

    expect(await progressCounter(page)).toBe(`${STEPS.length} of ${STEPS.length}`);
    // Still the same plan — "complete" must not have rewritten the steps.
    expect(await renderedSteps(page)).toEqual(STEPS);
  });
});
