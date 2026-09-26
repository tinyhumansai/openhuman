/**
 * Plan-mode review — the live turn really parks, and the user's decision
 * really releases it.
 *
 * Matrix 4.2.7 is ✅ on RU + RI + VU, and the note says why that is not enough:
 * *"WD E2E (agent-driven park flow) tracked as follow-up"*. The three existing
 * layers each own one end of the mechanism and none of them owns the join:
 *
 *   RU  `agent/plan_review/gate.rs`   — the oneshot parks, resolves, TTL-rejects.
 *   RI  `tests/json_rpc_e2e.rs`       — `plan_review_decide` answers.
 *   VU  `aui/PlanReviewPart.test.tsx` — the card renders and calls the RPC.
 *
 * What nothing covers is a REAL turn held open on an in-memory oneshot
 * (`PlanReviewGate`, one per process, not a database row) while a browser paints
 * a card from a socket event, and released when that card's button resolves it.
 * Every one of those pieces can pass its own test while the turn stays parked
 * forever — and a parked turn is silent. No error, no log line saying why; the
 * user watches a spinner that never ends.
 *
 * Turns are driven over `openhuman.channel_web_chat` (see `helpers/chat-drive.ts`),
 * never by typing, so a composer change cannot be reported as a gate failure.
 *
 * The agent is scripted through the mock's keyword rules. Stage 1 matches the
 * prompt and answers with a `request_plan_review` tool call; stage 2 matches the
 * tool's own result string, which the core fixes verbatim
 * (`agent/plan_review/tool.rs`: "approved: the user approved the plan…",
 * "rejected: the user rejected the plan…", "revise: the user requested changes…").
 * That is what makes the approve/reject assertions real rather than cosmetic:
 * the branch the gate took has to reach the model for the next stage to fire.
 */
import { expect, type Locator, type Page, test } from '@playwright/test';

import {
  resetMock,
  selectedThreadId,
  sendTurn,
  setKeywordRules,
  startNewThread,
  upstreamBodies,
  waitForSelectedThreadId,
} from '../helpers/chat-drive';
import { bootAuthenticatedPage, dismissWalkthroughIfPresent } from '../helpers/core-rpc';

const USER_ID = 'pw-chat-plan-review';

const PROMPT = 'PLANME rebuild the index';
const SUMMARY = 'PLAN-SUMMARY-MARKER';
const EXECUTED = 'PLAN-EXECUTED-MARKER';
const ABANDONED = 'PLAN-ABANDONED-MARKER';
const REVISED = 'PLAN-REVISED-MARKER';
const FEEDBACK = 'FEEDBACK-MARKER-do-it-in-two-steps';

/** The core's own wording for each resolution, as the tool hands it back. */
const APPROVED_RESULT = 'approved: the user approved';
const REJECTED_RESULT = 'rejected: the user rejected';
const REVISE_RESULT = 'revise: the user requested changes';

const RULES = [
  {
    keyword: 'PLANME',
    toolCalls: [
      {
        name: 'request_plan_review',
        arguments: { summary: SUMMARY, steps: ['read the index', 'rewrite it'] },
      },
    ],
  },
  { keyword: APPROVED_RESULT, content: EXECUTED },
  { keyword: REJECTED_RESULT, content: ABANDONED },
  { keyword: REVISE_RESULT, content: REVISED },
];

const planCard = (page: Page): Locator => page.getByTestId('plan-review-card');
const approveButton = (page: Page): Locator =>
  page.locator('[data-analytics-id="plan-review-approve-once"]');
const rejectButton = (page: Page): Locator =>
  page.locator('[data-analytics-id="plan-review-deny"]');
const reviseButton = (page: Page): Locator =>
  page.locator('[data-analytics-id="plan-review-approve-always"]');
const sendFeedbackButton = (page: Page): Locator =>
  page.locator('[data-analytics-id="plan-review-send-feedback-submit"]');
const stopButton = (page: Page): Locator => page.getByTestId('stop-generation-button');

async function openChat(page: Page): Promise<void> {
  await bootAuthenticatedPage(page, USER_ID, '/chat');
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('chat-message-input')).toBeVisible({ timeout: 30_000 });
}

/** Send the scripted prompt and wait until the turn is genuinely parked. */
async function parkAReview(page: Page): Promise<string> {
  const threadId = await waitForSelectedThreadId(page);
  await sendTurn(page, threadId, PROMPT);
  await expect(planCard(page), 'the scripted plan review never reached the browser').toBeVisible({
    timeout: 60_000,
  });
  return threadId;
}

/** True once any upstream request body carries `needle`. */
async function upstreamSaw(needle: string): Promise<boolean> {
  return (await upstreamBodies()).some(body => body.includes(needle));
}

// Same reasoning as chat-thread-isolation.spec.ts: the first browser spec of a
// shard pays the app's cold start, and these turns additionally wait on a real
// parked gate. The assertions are unchanged; only the budget moves.
test.describe.configure({ timeout: 120_000 });

test.describe('Plan-mode review', () => {
  test.skip(true, 'request_plan_review is no longer on the chat orchestrator belt');

  test.beforeEach(async () => {
    await resetMock();
    await setKeywordRules(RULES);
  });

  test('a parked review holds the turn open and shows the plan, with no answer yet', async ({
    page,
  }) => {
    await openChat(page);
    await parkAReview(page);

    // The plan the agent asked about, not a generic card.
    await expect(planCard(page).getByText(SUMMARY, { exact: false })).toBeVisible({
      timeout: 15_000,
    });
    // The turn is still running — this is the property a unit test cannot hold.
    await expect(stopButton(page), 'a parked turn is still in flight').toBeVisible({
      timeout: 15_000,
    });
    // And the agent has NOT proceeded. If the gate failed open, stage 2 would
    // already have fired and this marker would be on screen.
    await expect(page.getByText(EXECUTED, { exact: false })).toHaveCount(0);
    expect(
      await upstreamSaw(APPROVED_RESULT),
      'nothing was approved, so no approval result may have reached the model'
    ).toBe(false);
  });

  test('Approve resumes the same turn and the agent executes', async ({ page }) => {
    await openChat(page);
    await parkAReview(page);

    await approveButton(page).click({ force: true });

    // The released turn runs stage 2 and finishes in the same thread.
    await expect(
      page.getByText(EXECUTED, { exact: false }).last(),
      'approving must resume the parked turn, not merely dismiss its card'
    ).toBeVisible({ timeout: 60_000 });
    expect(
      await upstreamSaw(APPROVED_RESULT),
      'the approve resolution must reach the model as the tool result'
    ).toBe(true);
    await expect(planCard(page)).toHaveCount(0);
  });

  test('Reject resumes the turn and the agent does not execute', async ({ page }) => {
    await openChat(page);
    await parkAReview(page);

    await rejectButton(page).click({ force: true });

    await expect(page.getByText(ABANDONED, { exact: false }).last()).toBeVisible({
      timeout: 60_000,
    });
    // The assertion that makes approve and reject different things rather than
    // two ways to close a card: the approval wording must never have been sent.
    expect(
      await upstreamSaw(REJECTED_RESULT),
      'the reject resolution must reach the model as the tool result'
    ).toBe(true);
    expect(
      await upstreamSaw(APPROVED_RESULT),
      'a rejected plan must never hand the model an approval'
    ).toBe(false);
    await expect(page.getByText(EXECUTED, { exact: false })).toHaveCount(0);
  });

  test('Revise sends the typed feedback to the model', async ({ page }) => {
    await openChat(page);
    await parkAReview(page);

    await reviseButton(page).click({ force: true });
    const feedback = page.getByTestId('plan-review-feedback');
    await expect(feedback).toBeVisible({ timeout: 10_000 });
    // A plain textarea the user types into, not the assistant-ui composer.
    await feedback.fill(FEEDBACK);
    await sendFeedbackButton(page).click({ force: true });

    await expect(page.getByText(REVISED, { exact: false }).last()).toBeVisible({ timeout: 60_000 });
    // "The textarea accepted text" is worthless if the text never leaves. The
    // point of this case is the payload, not the affordance.
    expect(
      await upstreamSaw(FEEDBACK),
      'the revision feedback must reach the model, not just the card'
    ).toBe(true);
  });

  test('a parked review stays with its own thread across a switch', async ({ page }) => {
    await openChat(page);
    const threadA = await parkAReview(page);

    await startNewThread(page);
    await expect.poll(async () => selectedThreadId(page), { timeout: 15_000 }).not.toBe(threadA);
    await expect(
      planCard(page),
      'a review parked on another thread must not offer a decision here'
    ).toHaveCount(0);

    const rowA = page.getByTestId(`thread-row-${threadA}`);
    await expect(rowA).toBeVisible({ timeout: 15_000 });
    await rowA.click({ force: true });
    await expect.poll(async () => selectedThreadId(page), { timeout: 15_000 }).toBe(threadA);

    // Still parked, and still decidable — coming back must not have orphaned it.
    await expect(planCard(page)).toBeVisible({ timeout: 20_000 });
    await approveButton(page).click({ force: true });
    await expect(page.getByText(EXECUTED, { exact: false }).last()).toBeVisible({
      timeout: 60_000,
    });
  });
});
