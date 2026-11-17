import { expect, type Page, test } from '@playwright/test';

import { resetMock, sendTurn, setKeywordRules, startNewThread } from '../helpers/chat-drive';
import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

/**
 * `elements/context-display` (the composer ring) and `elements/context-breakdown`
 * (the popover behind it), on the product chat path.
 *
 * Both are mounted by `features/conversations/aui/ContextUsage.tsx`, which
 * `AssistantUiChat.tsx:206` renders into the composer. The dev gallery imports
 * only `contextBreakdownSegments` — a pure function — so there is no
 * gallery shortcut here even if one were wanted.
 *
 * # What "empty state" actually is, and what it is not
 *
 * This slice was dispatched on the premise that the empty/missing case is
 * fragile, evidenced by `fix(aui): handle missing context usage data
 * gracefully` and `fix(aui): correct context usage display for empty state`
 * each landing several times. **Those commit messages do not describe their
 * diffs** — see the report accompanying this branch. Read end to end, the six
 * commits carrying those two titles changed: a doc comment, two import lines,
 * a prettier reformat, a layout wrapper `div`, and the removal of a
 * `if (threadId !== '__never__') return null;` debug line. Not one of them
 * touched the empty or missing-data branch. They are auto-commits
 * ("Auto-committed-on: macbook") with generated subjects.
 *
 * The empty branch is still worth covering, because it is real code — but it
 * is worth covering for what it does, which is not what the titles imply:
 *
 *   `ContextDisplayRoot` bails with `if (!hasUsage) return null`
 *   (`context-display.tsx:180`), and `hasUsage` is
 *   `current.usage !== undefined || totalTokens > 0`. `ContextUsage` builds
 *   `ringUsage` with `useMemo<TokenUsage>(() => ({ ... }))`, which is **always
 *   an object**. So `hasUsage` is always true through the product path and the
 *   bail is unreachable from here: a thread with no turn shows a `0%` ring, it
 *   does not hide the control.
 *
 * That is asserted below as the real empty state. If someone later makes the
 * ring disappear on an empty thread believing they are fixing this, the first
 * case fails and points them here.
 */

const USER_ID = 'pw-aui-context-usage';
const PROMPT = 'Summarise the context budget please.';
const REPLY = 'Context canary 9f2a: here is the summary.';

async function openChat(page: Page): Promise<void> {
  await bootAuthenticatedPage(page, USER_ID, '/chat');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('chat-message-input')).toBeVisible({ timeout: 30_000 });
}

/** The integer percent the ring is currently showing. */
async function ringPercent(page: Page): Promise<number | null> {
  const text = await page.getByTestId('composer-context-usage').first().innerText();
  const match = /(\d+)\s*%/.exec(text);
  return match ? Number(match[1]) : null;
}

test.describe('assistant-ui context usage on the chat path', () => {
  test.beforeEach(async () => {
    await resetMock();
  });

  test('a thread with no turn shows the ring at zero rather than hiding it', async ({ page }) => {
    await openChat(page);
    await startNewThread(page);

    const ring = page.getByTestId('composer-context-usage');
    await expect(ring).toBeVisible({ timeout: 30_000 });

    // Zero, not absent. `ContextDisplayRoot`'s `!hasUsage` bail cannot fire
    // through this caller because `ringUsage` is always an object.
    expect(await ringPercent(page)).toBe(0);

    // And the composer is still usable — the claim "handles missing context
    // usage data gracefully" is only worth anything if the surface around it
    // still works.
    await expect(page.getByTestId('chat-message-input')).toBeEnabled();
  });

  test('the breakdown popover opens on an empty thread instead of throwing', async ({ page }) => {
    await openChat(page);
    await startNewThread(page);

    await page.getByTestId('composer-context-usage').first().click();

    // `ContextUsage` fetches `agent.context_breakdown` only on open, and
    // renders a loading, ready or error body — never nothing, and never an
    // unhandled rejection. Any of the three is a pass here; what is being
    // pinned is that opening it on a thread with no usage produces a rendered
    // popover rather than an error boundary.
    const popover = page.getByTestId('composer-token-breakdown');
    await expect(popover).toBeVisible({ timeout: 20_000 });
    await expect(popover).not.toBeEmpty();

    // The chat surface survived it.
    await expect(page.getByTestId('chat-message-input')).toBeEnabled();
  });

  test('the breakdown popover renders the section rows for a turn that has usage', async ({
    page,
  }) => {
    await setKeywordRules([{ keyword: PROMPT, content: REPLY }]);

    await openChat(page);
    const threadId = await startNewThread(page);
    await sendTurn(page, threadId, PROMPT);
    await expect(page.getByText(REPLY).last()).toBeVisible({ timeout: 60_000 });

    await page.getByTestId('composer-context-usage').first().click();
    const popover = page.getByTestId('composer-token-breakdown');
    await expect(popover).toBeVisible({ timeout: 20_000 });

    // `contextBreakdownSegments` always emits these three rows, in this order.
    // Output is not a separate prompt partition: provider output may include
    // replies trimmed from the final request, so the breakdown counts only
    // system prompt, tool schemas, and the remaining conversation.
    await expect(popover).toContainText('System prompt');
    await expect(popover).toContainText('Tool schemas');
    await expect(popover).toContainText('Your input');
  });
});
