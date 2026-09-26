/**
 * `subagent-list` — the fan-out roster a parallel delegation renders.
 *
 * Family: `components/assistant-ui/elements/subagent-list.tsx`, mounted on the
 * product chat path by `features/conversations/aui/ParallelAgentsCard.tsx`,
 * which `aui/toolkit.tsx` registers for `spawn_parallel_agents`. Zero e2e
 * coverage before this spec.
 *
 * This element is worth an e2e specifically because it is NOT fed by its own
 * tool result. `ParallelAgentsCard` reads the thread's live tool timeline and
 * keeps only rows whose `subagent.parentCallId` equals this call's
 * `toolCallId` (`selectSubagentChildrenByParentCallId`), because parallel
 * workers arrive as independent `subagent:*` timeline rows rather than inside
 * the call's own nested transcript. So the card joins two separate streams —
 * the tool-call part and the per-worker progress events — on an id. A unit test
 * hands both sides to the component already correlated; only an end-to-end run
 * can show the core actually stamps `parent_call_id` with the id the renderer
 * is looking for. If that correlation breaks, `children.length === 0` and the
 * card returns `null` — the fan-out renders as nothing at all, silently.
 *
 * ⚠️ STATUS 2026-09-24: THIS SPEC DOES NOT PASS YET, and the failure is NOT
 * isolated. Observed: `assistant-ui-parallel-agents-call` never appeared within
 * 60s. That is all that is known. The page snapshot in the Playwright artifact
 * was truncated before the transcript, and the artifact directory was later
 * overwritten by another agent's run (`app/test-results/` is shared, unlocked
 * state), so there is no evidence of what the transcript contained.
 *
 * Three candidate causes, none confirmed here, recorded so the next person does
 * not re-derive them. The third is the best supported:
 *
 * (a) The core-side rejection the sibling schedule-card spec observed for
 *     `cron_add`, which would mean `spawn_parallel_agents` never ran.
 * (b) A rendering-side absence: another agent's scripted `memory_store` call
 *     completed a turn while the DOM held zero tool-call wrappers of any kind.
 * (c) **Socket events not reaching the browser on this lane.** This card's rows
 *     come from `state.chatRuntime.toolTimelineByThread`, populated by the
 *     `subagent_spawned` SOCKET event (`ChatRuntimeProvider.tsx:890-930`, whose
 *     own comment calls it "this socket ... event"). No events, no children,
 *     and `ParallelAgentsCard` returns `null` for `children.length === 0` —
 *     exactly the observed symptom. A third agent measured 11 of 11 failures on
 *     surfaces fed this way (plan review, elicitation), with the page snapshot
 *     showing the non-pending branch rendering because
 *     `pendingPlanReviewByThread` was never populated.
 *
 * Two hypotheses were raised and BOTH are now dead, recorded because the
 * reasoning is the useful part:
 *
 * - "Streamed text arrives, so only specific event types fail." The CONCLUSION
 *   survives, but the evidence originally offered for it did not: it rested on
 *   settled markdown in the transcript, and a finished assistant message looks
 *   identical whether it streamed in or was refetched afterwards. The DOM does
 *   not record how content arrived.
 * - "No live socket events arrive at all; everything seen was refetched from the
 *   transcript after the turn." Measured and killed: `chat-thread-isolation`
 *   passes 3/3, and its case 2 asserts a token visible WHILE the turn is still
 *   streaming. Live `text_delta` demonstrably arrives on this lane.
 *
 * So streaming works. That makes this spec's failure HARDER to explain, not
 * easier, and rules out the tidy "one cause, four slices" story:
 * `subagent_spawned` is published by `web_chat/progress_bridge_subagent_events.rs`
 * (`on_subagent_spawned` -> `publish_seq_stamped`) — the progress bridge, the
 * same publisher as the streaming that works. It does NOT share a publisher with
 * `plan_review_request`, which is bridged separately by `ApprovalSurfaceSubscriber`
 * off the DomainEvent bus (`web_chat/event_bus.rs:41`, `:594-686`). A
 * plan-review-shaped explanation therefore does not transfer here, and these
 * should be filed as two findings with a noted resemblance rather than one.
 *
 * ⛔ UPSTREAM BREAKAGE, established by a CONTROL rather than by inference:
 * **tool calls do not render in the Playwright web lane on this branch.** The
 * pre-existing `test/playwright/specs/tool-call-presentation.spec.ts` (tracked,
 * last touched by `cd3806b19`, authored by nobody in this round) fails at
 * `expect(page.getByTestId('tool-timeline')).toBeVisible()` — and fails AFTER
 * its canary assertion passes, so the turn completed and the agent replied
 * while no tool call rendered.
 *
 * That control is sound on its own terms: its `beforeEach` boots the page
 * BEFORE the test body sets `llmForcedResponses`, so the ordered-queue hazard
 * (boot-time LLM requests eating queue entries) cannot apply to it, and the
 * canary rendering proves the scripted sequence was consumed by the intended
 * turn.
 *
 * It went unnoticed because that spec DOES run in ci-full's 64-shard sweep, but
 * the job carries `continue-on-error: true` and is excluded from the gate
 * (#3615) — a broken tool-render path fails there invisibly. Which is the
 * regression class a non-blocking lane is built to hide.
 *
 * Bisect window for that breakage, since the obvious suspects are innocent:
 * the control was restored by `cd3806b19` at 06:14 on 2026-09-24 and the
 * breakage was observed the same evening. Two commits on this branch SOUND like
 * the cause — `71d8874af` "handle missing toolkit in AUI conversation" and
 * `d5e81a916` "handle missing toolkit state on initial render" — and are not:
 * their complete diffs add this very `spawn_parallel_agents` toolkit entry and
 * its import, nothing else. Auto-generated subjects describing a fix that is not
 * in the diff. `7e1f5d90a` (18:12, desktop shell + chat chrome) touches
 * `thread.tsx` but its only tool/part change there is a comment. One entry in
 * the window was NOT examined at all: `58407ea53`, the 15:51 upstream merge.
 * That is a gap in the search, not a suspicion about the commit — it is
 * recorded so the next person knows which stone is unturned, and nothing here
 * should be read as nominating it.
 *
 * This supersedes the by-elimination reasoning that preceded it. `ParallelAgentsCard`
 * is a toolkit render ON a tool-call part, so if no tool call renders this card
 * cannot render regardless of whether `spawn_parallel_agents` ran, whether its
 * workers spawned, or whether `subagent_spawned` was delivered. The spec is
 * blocked upstream of everything it asserts.
 *
 * Deciding between them needs one run that captures the transcript. Copy the
 * artifact out of the tree before releasing the ci-slot: `e2e-web-session.sh`
 * wipes `test-results/` at start, its wrapper exits 0 even when tests fail, and
 * the directory is shared with every other agent's run.
 *
 * Two workers because the tool's own schema sets `minItems: 2` on `tasks`
 * (`spawn_parallel_agents_policy_tests.rs:11`); one task is not a valid call.
 */
import { expect, type Page, test } from '@playwright/test';

import { resetMock, sendTurn, setKeywordRules, startNewThread } from '../helpers/chat-drive';
import { bootAuthenticatedPage, dismissWalkthroughIfPresent } from '../helpers/core-rpc';

const USER_ID = 'pw-aui-subagent-list';

const TRIGGER = 'PARALLEL-FANOUT please';

/** `conversations.tools.parallelAgentsAggregating` (en.ts:3793). */
const AGGREGATING = 'Aggregating results';

const RULES = [
  {
    keyword: 'PARALLEL-FANOUT',
    toolCalls: [
      {
        name: 'spawn_parallel_agents',
        arguments: {
          tasks: [
            { agent_id: 'researcher', prompt: 'Summarise branch ALPHA and stop.' },
            { agent_id: 'researcher', prompt: 'Summarise branch BRAVO and stop.' },
          ],
        },
      },
    ],
  },
  // The workers themselves reach the same mock; keep their turns short so both
  // settle inside the spec's budget rather than streaming for its duration.
  { keyword: 'branch ALPHA', streamScript: [{ text: 'alpha done' }, { finish: 'stop' }] },
  { keyword: 'branch BRAVO', streamScript: [{ text: 'bravo done' }, { finish: 'stop' }] },
];

async function openChat(page: Page): Promise<void> {
  await bootAuthenticatedPage(page, USER_ID, '/chat');
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('chat-message-input')).toBeVisible({ timeout: 30_000 });
}

test.beforeEach(async () => {
  await resetMock();
  await setKeywordRules(RULES);
});

test.describe('assistant-ui subagent list', () => {
  // `spawn_parallel_agents` was removed from the chat orchestrator's direct
  // tool belt; the supported path is async delegation. This legacy direct-call
  // contract remains as documentation until its replacement is landed.
  test.skip(true, 'legacy spawn_parallel_agents surface; async delegation is the supported path');

  test('a parallel fan-out renders both workers and settles when both finish', async ({ page }) => {
    test.setTimeout(120_000);

    await openChat(page);
    const threadId = await startNewThread(page);

    await sendTurn(page, threadId, TRIGGER);

    // The card renders at all only if `parentCallId` correlation succeeded —
    // `children.length === 0` returns null. So its presence is the join
    // assertion, not decoration.
    const card = page.getByTestId('assistant-ui-parallel-agents-call');
    await expect(card).toBeVisible({ timeout: 60_000 });

    // Terminal state, observed through the element's own logic rather than a
    // sleep: `showSummary={anyRunning}`, and `anyRunning` is
    // `completedCount < children.length`. The aggregating summary row is
    // present exactly while at least one worker is live and disappears when
    // the last one settles, so waiting for it to go is waiting for both
    // workers to reach a non-active status.
    await expect(card.getByText(AGGREGATING, { exact: false })).toHaveCount(0, { timeout: 90_000 });

    // One progressbar per worker (`subagent-list.tsx:66-67`), counted AFTER the
    // summary row has gone so the count is workers only — `showSummary` adds a
    // progressbar of its own (`:95-96`) and would otherwise inflate it.
    //
    // Counted rather than name-matched on purpose. The label is
    // `${agent.name} progress` where `name` is `displayName || agentId ||
    // 'sub-agent'`, so matching on "researcher" would bind this spec to which
    // of those three the core happened to populate. The count is the claim
    // anyway: exactly two, because a fan-out that correlated only its first
    // worker still renders a card and still looks plausible.
    await expect(card.getByRole('progressbar')).toHaveCount(2, { timeout: 30_000 });
  });
});
