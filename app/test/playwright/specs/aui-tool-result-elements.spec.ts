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
 * These assertions use the current shipped toolkit: FileBody renders edits,
 * ToolDataView renders flat tabular results, and DocumentArtifactCall renders
 * generated document titles. They remain separate from grouped task insights.
 */
import { expect, type Locator, type Page, test } from '@playwright/test';
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';

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
const GITHUB_REPOSITORY_ACTION = 'GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER';

function makeComposioTool(name: string) {
  return {
    type: 'function',
    function: {
      name,
      description: 'List repositories for the authenticated user',
      parameters: { type: 'object', properties: {} },
    },
  };
}

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

function captureToolProgress(page: Page): { events: Record<string, unknown>[] } {
  const events: Record<string, unknown>[] = [];
  page.on('websocket', socket => {
    socket.on('framereceived', ({ payload }) => {
      if (typeof payload !== 'string') return;
      const frame = payload.slice(payload.indexOf('['));
      try {
        const decoded: unknown = JSON.parse(frame);
        if (!Array.isArray(decoded) || typeof decoded[0] !== 'string') return;
        const name = decoded[0];
        if (name !== 'tool_call' && name !== 'tool_result') return;
        const data = decoded[1];
        if (!data || typeof data !== 'object') return;
        const event = data as Record<string, unknown>;
        const sanitized = {
          name,
          request_id: event.request_id,
          round: event.round,
          tool_call_id: event.tool_call_id,
          tool_name: event.tool_name,
          success: event.success,
        };
        events.push(sanitized);
        console.log('[epic235 socket tool progress]', JSON.stringify(sanitized));
      } catch {
        // Engine.IO control frames and non-JSON packets carry no tool progress.
      }
    });
  });
  return { events };
}

async function waitForTurnCompletion(page: Page, threadId: string): Promise<void> {
  await expect
    .poll(
      () =>
        page.evaluate(id => {
          const store = (
            window as unknown as {
              __OPENHUMAN_STORE__?: {
                getState?: () => {
                  chatRuntime?: { inferenceTurnLifecycleByThread?: Record<string, string> };
                };
              };
            }
          ).__OPENHUMAN_STORE__;
          return store?.getState?.().chatRuntime?.inferenceTurnLifecycleByThread?.[id] ?? null;
        }, threadId),
      { timeout: 45_000, message: 'the synthetic tool turn did not complete' }
    )
    .toBeNull();
}

async function captureToolTimeline(page: Page, threadId: string): Promise<unknown> {
  return page.evaluate(id => {
    const store = (
      window as unknown as {
        __OPENHUMAN_STORE__?: {
          getState?: () => {
            chatRuntime?: {
              toolTimelineByThread?: Record<
                string,
                Array<{
                  id: string;
                  name: string;
                  status: string;
                  round: number;
                  argsBuffer?: string;
                  result?: string;
                }>
              >;
            };
          };
        };
      }
    ).__OPENHUMAN_STORE__;
    return store
      ?.getState?.()
      .chatRuntime?.toolTimelineByThread?.[
        id
      ]?.map(row => ({ id: row.id, name: row.name, status: row.status, round: row.round, argsLength: row.argsBuffer?.length ?? 0, resultLength: row.result?.length ?? 0 }));
  }, threadId);
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
async function openActivityGroup(page: Page): Promise<Locator> {
  const group = page.locator('[data-slot="tool-group-root"]').last();
  await expect(group, 'the assistant response has no activity disclosure').toBeVisible({
    timeout: 30_000,
  });
  const trigger = group.locator('[data-slot="tool-group-trigger"]');
  // sendTurn returns when the RPC is accepted, while this group is often still
  // open because the turn is live. Toggle it closed and open again so the
  // interaction records an explicit user choice; it then remains open when the
  // response settles and ActivityGroup's live default changes to closed.
  if ((await trigger.getAttribute('aria-expanded')) === 'true') {
    await trigger.click();
    await expect(trigger).toHaveAttribute('aria-expanded', 'false', { timeout: 10_000 });
  }
  await trigger.click();
  await expect(trigger).toHaveAttribute('aria-expanded', 'true', { timeout: 10_000 });
  return group;
}

async function openToolCall(page: Page, expectedLabel: string | RegExp): Promise<Locator> {
  const group = await openActivityGroup(page);
  const card = group.getByTestId('assistant-ui-tool-call').filter({ hasText: expectedLabel });
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
    await setMockBehavior(
      'composioTools',
      JSON.stringify([makeComposioTool(GITHUB_REPOSITORY_ACTION)])
    );
  });

  // ── elements/code-diff.tsx ────────────────────────────────────────────

  test('an edit renders its added and removed lines through the code-diff element', async ({
    page,
  }) => {
    const progress = process.env.EPIC235_CAPTURE_TOOL_PROGRESS ? captureToolProgress(page) : null;
    const threadId = await openChat(page);
    const PROMPT = 'AUI-DIFF-CASE raise the retry count and the timeout';
    const actionDir = process.env.OPENHUMAN_ACTION_DIR;
    if (!actionDir) throw new Error('The web E2E session did not provide OPENHUMAN_ACTION_DIR');
    const fixturePath = path.join(actionDir, 'e2e/aui/diff-subject.ts');
    await mkdir(path.dirname(fixturePath), { recursive: true });
    await writeFile(fixturePath, 'const retries = 1;\nconst timeoutMs = 500;\n');
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
    await waitForTurnCompletion(page, threadId);
    if (progress) {
      console.log(
        '[epic235 edit progress]',
        JSON.stringify({
          events: progress.events,
          timeline: await captureToolTimeline(page, threadId),
        })
      );
    }
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
    const progress = process.env.EPIC235_CAPTURE_TOOL_PROGRESS ? captureToolProgress(page) : null;
    const threadId = await openChat(page);
    // `ToolDataView` renders `DataTable` only for a non-empty array of plain
    // objects that share one key set and hold only primitives
    // (`ToolDataView.tsx:isFlatObjectArray`). `result` is one of the semantic
    // keys it unwraps to before deciding, so this payload reaches the table.
    await setMockBehavior(
      `composioExecuteResponse_${GITHUB_REPOSITORY_ACTION}`,
      JSON.stringify({
        result: [
          { repo_name: 'openhuman', open_issues: 42, archived: false },
          { repo_name: 'tinybus', open_issues: 7, archived: false },
        ],
      })
    );
    await setKeywordRules([
      {
        keyword: 'AUI-TABLE-CASE',
        toolCalls: [{ name: 'tool_search', arguments: { query: 'list GitHub repositories' } }],
      },
      {
        // Match the result's stable action identity, not a hit count. The
        // query can legitimately return multiple nearby GitHub actions, and
        // the count is a ranking detail rather than the action we intend to
        // exercise.
        keyword: GITHUB_REPOSITORY_ACTION,
        toolCalls: [{ name: GITHUB_REPOSITORY_ACTION, arguments: { limit: 2 } }],
      },
    ]);
    await setMockBehavior('llmStreamChunkDelayMs', '10');

    await sendTurn(page, threadId, 'AUI-TABLE-CASE list my repositories');
    await waitForTurnCompletion(page, threadId);
    if (progress) {
      console.log(
        '[epic235 tool progress]',
        JSON.stringify({
          events: progress.events,
          timeline: await captureToolTimeline(page, threadId),
        })
      );
    }
    const card = await openToolCall(
      page,
      /GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER|repositor/i
    );

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
    // `generate_document` is registered as `display: standalone`; assistant-ui
    // deliberately leaves it outside the normal activity disclosure group.
    // Scope the element through its actual assistant-message content instead.
    const artifactMessage = page
      .locator('[data-slot="aui_assistant-message-content"]')
      .filter({ has: page.locator('[data-slot="artifact-card"]') })
      .last();
    const card = artifactMessage.locator('[data-slot="artifact-card"]').last();
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
