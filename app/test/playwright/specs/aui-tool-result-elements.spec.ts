/**
 * assistant-ui tool-result presentation elements, end to end.
 *
 * Covers three vendored element families that had **zero** e2e coverage before
 * this file (`git grep -l 'code-diff\|data-table\|artifact-card'` over
 * `test/playwright/specs` and `test/e2e/specs` returned nothing):
 *
 * | Element | Real importer on the product chat path | What makes it appear |
 * |---|---|---|
 * | `elements/code-diff.tsx` | `features/conversations/tools/ToolBodies.tsx` `FileBody` | an `edit` tool call carrying `old_string` / `new_string` |
 * | `elements/data-table.tsx` | `features/conversations/tools/ToolDataView.tsx` | a tool result that is a flat, uniform object array |
 * | `elements/artifact-card.tsx` | `features/conversations/aui/MediaAndDocumentCalls.tsx` `DocumentArtifactCall` | a `generate_document` / `generate_presentation` call |
 *
 * **Not** driven through the dev-only gallery at `/dev/tools`
 * (`pages/dev/ToolCallGallery.tsx`, "Registered only in dev builds"). That
 * route imports several of these families and is the easy way to make them
 * render, but it proves nothing about the product: it mounts the elements with
 * hand-written props instead of letting a tool result reach them. Every
 * assertion below goes through a real turn — the mock LLM emits a tool call,
 * the core runs it, and the chat renders the result.
 *
 * Turns go over `openhuman.channel_web_chat` via `helpers/chat-drive.ts`, never
 * the composer: `ComposerTextBridge` used to take the chat surface to its error
 * boundary when driven at speed, and a spec about a tool result should not be
 * able to fail for a composer reason.
 *
 * No real backend or third-party calls: the mock LLM is scripted with
 * `llmKeywordRules`, and the Composio action's payload comes from the mock's own
 * `composioExecuteResponse_<ACTION>` behaviour knob.
 *
 * ## STATUS 2026-09-24: these cases are RED, and not because of what they assert
 *
 * Every case here fails at `openToolCall`, before reaching a single element
 * assertion: the round settles, the agent replies, and no `assistant-ui-tool-call`
 * card appears. Three independent legs say that is not a claim about this file.
 *
 * 1. **A control on a spec this file does not touch.**
 *    `test/playwright/specs/tool-call-presentation.spec.ts` — restored hours
 *    earlier in `cd3806b19` — fails at its line 151,
 *    `expect(page.getByTestId('tool-timeline')).toBeVisible()`, AFTER its canary
 *    assertion passes. It drives via the COMPOSER (`sendMessage`, lines 108-115:
 *    `chat-message-input.fill` then `send-message-button.click`), not this file's
 *    RPC driver, so it exercises the real send path.
 *
 * 2. **The `chat-drive.ts` lifecycle bug is NOT the cause.** A peer session found
 *    that `sendTurn` never dispatched `chatRuntime/beginInferenceTurn`, which a
 *    real composer send does (`Conversations.tsx:1161`), leaving `isRunning`
 *    false for RPC-driven turns. That was a genuine harness defect and it is
 *    fixed (`armTurnLifecycle`). **These four cases were re-run on the fixed
 *    driver and are still red, with the same message.** Leg 1 was never affected
 *    by it either, being composer-driven.
 *
 * 3. **A second element family, confound removed.** The same peer's `todo` spec
 *    is still red on the fixed driver with its UPSTREAM stage passing —
 *    `upstreamBodies()` grew past `before + 1`, so the tool call went out, the
 *    result came back, and the harness called the model again. Tool round trip
 *    yes, render no.
 *
 * ### Scope: wider than "tool calls do not render"
 *
 * That framing is narrower than the evidence. The peer's pinned `todo-checklist`
 * also failed, and it is not a tool-call part at all — it renders off the
 * `thread_todos_changed` socket event via `useThreadTodos` (`useThreadTodos.ts:3`;
 * `TodoListPart.tsx:68` contrasts the two paths explicitly). So **at least two
 * independent render paths fail: tool-call parts, and a socket-event-driven
 * pinned surface.** Whether they share a root cause is UNKNOWN and is not
 * asserted here.
 *
 * What CAN be said bounds the search without inventing a cause: the two paths
 * converge at exactly one place. `ChatRuntimeProvider.tsx` handles both the
 * tool-call stream (`:806-1050`, `toolCallReceived`) and `thread_todos_changed`
 * (`:1461`), and they share nothing below it — the tool-call part renders
 * through the assistant-ui toolkit, the checklist through `useThreadTodos`. So
 * IF one defect explains both, it is at or above that handler; if it is below,
 * there are two. That is a constraint on where to look, not a claim about which
 * is true. (Bound suggested by a peer session, verified here against the
 * provider source.)
 *
 * **These cases are therefore written but UNVERIFIED, and none is revert-proven.**
 * Proving a fault against an already-red spec establishes nothing, so that was not
 * attempted. When the lane renders tool calls again, run this file first: if a case
 * still fails it will fail at its own assertion rather than at `openToolCall`, and
 * that failure is then about the element.
 */
import { expect, type Locator, type Page, test } from '@playwright/test';

import {
  resetMock,
  sendTurn,
  setKeywordRules,
  setMockBehavior,
  startNewThread,
  waitForConnectedSocketId,
} from '../helpers/chat-drive';
import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const USER_ID = 'pw-aui-tool-result-elements';

async function openChat(page: Page): Promise<string> {
  await bootAuthenticatedPage(page, USER_ID, '/chat');
  await page.goto('/#/chat');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('chat-message-input')).toBeVisible();
  await waitForConnectedSocketId(page);
  return startNewThread(page);
}

/**
 * Script the mock LLM to answer ONE prompt with a tool call.
 *
 * Keyword-matched (`llmKeywordRules`), deliberately not the `llmForcedResponses`
 * queue. The queue is consumed in order by *every* LLM request the process
 * makes, and booting a page and opening a thread makes some of its own before
 * the turn under test — so entry #1 (the tool call) gets eaten and the turn
 * receives entry #2. The symptom is nasty: the round completes, a reply
 * appears, and no tool call ever happens. The first revision of this file used
 * the queue and all four cases failed that way. A keyword rule fires on the
 * prompt text instead, so it cannot be consumed by an unrelated request.
 *
 * No canary: the follow-up response falls through to the mock's default
 * (`llm.mjs:505-516`), and waiting for the rendered body is the better settle
 * signal anyway — `AssistantUiToolCall` mounts `richBody` only once the call is
 * no longer running.
 */
async function scriptToolCall(
  keyword: string,
  toolCalls: { name: string; arguments: Record<string, unknown> }[]
): Promise<void> {
  await setKeywordRules([{ keyword, toolCalls }]);
  await setMockBehavior('llmStreamChunkDelayMs', '10');
}

/**
 * Wait for the round to have produced a tool card at all, then open it.
 *
 * Staged deliberately. The bodies below live inside a Radix
 * `CollapsibleContent` (`elements/tool-call.tsx:77-119`), which UNMOUNTS its
 * children while closed, and a settled tool card starts closed —
 * `useDisclosure(key, awaitingUser)` defaults to `awaitingUser`, which is
 * `false` for an ordinary call (`AssistantUiToolCall.tsx:100-102`). So "element
 * not found" has two very different causes: the tool never ran, or it ran and
 * the card is shut. Asserting the card first separates them, instead of
 * reporting a core problem as a rendering one.
 *
 * The trigger is identified by `aria-expanded`, which Radix owns, rather than
 * "the first button in the card" — an earlier revision clicked by button order
 * and silently opened nothing.
 */
async function openToolCall(page: Page, expectedLabel: string | RegExp): Promise<Locator> {
  const card = page.getByTestId('assistant-ui-tool-call').filter({ hasText: expectedLabel });
  await expect(
    card.first(),
    `the round settled but produced no tool card matching ${String(expectedLabel)} — ` +
      'the core did not run the scripted tool call, so nothing downstream can be about rendering'
  ).toBeVisible({ timeout: 30_000 });

  const target = card.last();
  const trigger = target.locator('[aria-expanded]').first();
  if ((await trigger.getAttribute('aria-expanded')) === 'false') {
    await trigger.click({ force: true });
  }
  // Positive confirmation that the click landed. Without it a missed trigger
  // reads downstream as "the element does not render".
  await expect(trigger).toHaveAttribute('aria-expanded', 'true', { timeout: 10_000 });
  return target;
}

test.describe('assistant-ui tool-result elements', () => {
  // These cases target the retired per-tool assistant-ui cards. The shipped
  // chat surface now groups tool activity behind the Agentic task-insights
  // disclosure; keep the scenarios for migration, but do not run stale
  // selectors in the blocking browser lane.
  test.skip(true, 'legacy per-tool cards replaced by the Agentic task-insights surface');

  test.beforeEach(async () => {
    await resetMock();
    // The Composio case below calls a GitHub action. Without a seeded toolkit
    // and an ACTIVE connection the action is not reachable and the round would
    // settle on an error body rather than a tabular result — the same seeding
    // `harness-composio-tool-flow.spec.ts` does.
    await setMockBehavior('composioToolkits', JSON.stringify(['github']));
    await setMockBehavior(
      'composioConnections',
      JSON.stringify([{ id: 'conn-github', toolkit: 'github', status: 'ACTIVE' }])
    );
  });

  // ── elements/code-diff.tsx ────────────────────────────────────────────

  test('an edit renders its added and removed lines through the code-diff element', async ({
    page,
  }) => {
    const threadId = await openChat(page);
    const PROMPT = 'AUI-DIFF-CASE raise the retry count and the timeout';
    await scriptToolCall('AUI-DIFF-CASE', [
      {
        name: 'edit',
        arguments: {
          path: 'e2e/aui/diff-subject.ts',
          old_string: 'const retries = 1;\nconst timeoutMs = 500;',
          new_string: 'const retries = 5;\nconst timeoutMs = 2000;',
        },
      },
    ]);

    await sendTurn(page, threadId, PROMPT);
    const card = await openToolCall(page, /Edit|edit/);

    const diff = card.getByTestId('tool-body-file-diff').first();
    await expect(diff).toBeVisible({ timeout: 20_000 });

    // The filename, shortened by `shortenPath`, identifies which file changed.
    await expect(diff).toContainText('diff-subject.ts');

    // Both sides render, and each carries its own text. A diff that dropped
    // the removed side, or rendered the old text on both, still shows a card.
    await expect(diff).toContainText('const retries = 1;');
    await expect(diff).toContainText('const retries = 5;');
    await expect(diff).toContainText('const timeoutMs = 500;');
    await expect(diff).toContainText('const timeoutMs = 2000;');

    // The counts must agree with the payload: two lines removed, two added.
    // `FileBody` derives them from the args it was handed
    // (`ToolBodies.tsx:214-226`), so a header that disagrees with the body is
    // the miscount this pins.
    await expect(diff).toContainText('+2');
    await expect(diff).toContainText('−2');
  });

  test('an edit that changes nothing still reports a diff of its own size', async ({ page }) => {
    const threadId = await openChat(page);
    const PROMPT = 'AUI-NOOP-CASE rewrite that line to the same thing';
    const unchanged = 'export const VERSION = 3;';
    await scriptToolCall('AUI-NOOP-CASE', [
      {
        name: 'edit',
        arguments: {
          path: 'e2e/aui/noop-subject.ts',
          old_string: unchanged,
          new_string: unchanged,
        },
      },
    ]);

    await sendTurn(page, threadId, PROMPT);
    const card = await openToolCall(page, /Edit|edit/);

    const diff = card.getByTestId('tool-body-file-diff').first();
    await expect(diff).toBeVisible({ timeout: 20_000 });

    // This pins CURRENT behaviour, which is worth being explicit about: an
    // edit whose old and new text are identical is rendered as a full
    // removal plus a full re-addition (`+1 −1`), not as unchanged context.
    // `FileBody` never compares the two sides — it maps `old_string` to
    // removed lines and `new_string` to added lines unconditionally
    // (`ToolBodies.tsx:213-217`), so there is no "no change" branch to reach.
    //
    // I am pinning it rather than asserting what it arguably should do,
    // because a spec that asserted "renders as context" would be red against
    // shipped code and would be describing a feature request. The judgement
    // that this reads wrong is in the W5 report as a candidate issue, not
    // encoded here as a failing test.
    await expect(diff).toContainText('+1');
    await expect(diff).toContainText('−1');
    await expect(diff).toContainText(unchanged);
  });

  // ── elements/data-table.tsx ───────────────────────────────────────────

  test('a tabular tool result renders its rows and columns through the data-table element', async ({
    page,
  }) => {
    const threadId = await openChat(page);
    // `ToolDataView` renders `DataTable` only for a non-empty array of plain
    // objects that share one key set and hold only primitives
    // (`ToolDataView.tsx:isFlatObjectArray`). `result` is one of the semantic
    // keys it unwraps to before deciding, so this payload reaches the table.
    await setMockBehavior(
      'composioExecuteResponse_GITHUB_LIST_REPOS',
      JSON.stringify({
        result: [
          { repo_name: 'openhuman', open_issues: 42, archived: false },
          { repo_name: 'tinybus', open_issues: 7, archived: false },
        ],
      })
    );
    await scriptToolCall('AUI-TABLE-CASE', [
      { name: 'GITHUB_LIST_REPOS', arguments: { limit: 2 } },
    ]);

    await sendTurn(page, threadId, 'AUI-TABLE-CASE list my repositories');
    const card = await openToolCall(page, /GITHUB_LIST_REPOS|repositor/i);

    const table = card.locator('[data-slot="data-table"]').first();
    await expect(table).toBeVisible({ timeout: 20_000 });

    // Columns come from the first row's keys, run through `friendlyLabel`
    // (`ToolDataView.tsx:flatRowColumns`), so the header is derived from the
    // payload rather than hard-coded: `repo_name` must render as "Repo name".
    await expect(table).toContainText('Repo name');
    await expect(table).toContainText('Open issues');
    await expect(table).toContainText('Archived');

    // Both rows, with their own cell values. Asserting both is what catches a
    // table that renders only the first row.
    await expect(table).toContainText('openhuman');
    await expect(table).toContainText('42');
    await expect(table).toContainText('tinybus');
    await expect(table).toContainText('7');

    // A boolean cell is stringified by `flatRowColumns`, not dropped.
    await expect(table).toContainText('false');
  });

  // ── elements/artifact-card.tsx ────────────────────────────────────────

  test('a generated document shows its own title, not the generic kind placeholder', async ({
    page,
  }) => {
    const threadId = await openChat(page);
    const DOC_TITLE = 'Q3 Infrastructure Review';
    await scriptToolCall('AUI-DOC-CASE', [
      {
        name: 'generate_document',
        arguments: { title: DOC_TITLE, prompt: 'summarise the infrastructure work this quarter' },
      },
    ]);

    await sendTurn(page, threadId, 'AUI-DOC-CASE write up the infrastructure review');
    const toolCard = await openToolCall(page, /Document|document/);

    const card = toolCard.locator('[data-slot="artifact-card"]').first();
    await expect(card).toBeVisible({ timeout: 20_000 });

    // The point of this assertion. `DocumentArtifactCall` resolves the title as
    // `result.title ?? args.title ?? kindTitle` (`MediaAndDocumentCalls.tsx:131-140`),
    // where `kindTitle` is the generic "Document" / "Presentation" string. If
    // both lookups break the card still renders, still looks right, and shows a
    // placeholder — which is exactly the failure a "does the card appear" spec
    // cannot see. So: the artifact's own title must be present, and the bare
    // placeholder must not be the card's title.
    await expect(card).toContainText(DOC_TITLE);

    // `toHaveText('Document')` on the card would never fail — the card's full
    // text also carries the meta line, so it can never equal the placeholder.
    // The title is its own node (`artifact-card.tsx:110`), so assert on that:
    // it must read the artifact's title and not the generic kind string.
    const cardTitle = card.locator('p.truncate').first();
    await expect(cardTitle).toHaveText(DOC_TITLE);
  });
});
