/**
 * The user-message action bar offers only what the runtime can honour (#5897).
 *
 * # What went wrong, and why a DOM test is the guard
 *
 * `useOpenHumanExternalStore` supplies `onNew` / `onCancel` and implements
 * neither `onEdit` nor `setMessages`, so assistant-ui reports `edit: false` and
 * `EditComposer` never renders. `ActionBarPrimitive.Edit` was rendered anyway,
 * so every user message carried a pencil button that was visible, hoverable,
 * clickable — and completely inert.
 *
 * The capability gate for this already existed. `useAuiEditCapabilities`
 * (`features/conversations/components/aui/auiThreadState.ts`) calls itself "the
 * honest gate for those affordances" and had **zero production consumers**, and
 * the same file states the contract: *"deliberately absent rather than
 * rendered-and-inert: an edit button that looks supported and silently does
 * nothing is worse than no button."*
 *
 * `auiThreadState.test.tsx` asserts the capability FLAG and passes. Nobody ever
 * asserted the DOM, which is exactly how this shipped with the guard apparently
 * in place — so the guard has to live at the DOM, in a browser, which is what
 * this file is.
 *
 * # Scope
 *
 * These assert the *current* contract: while the adapter cannot edit, the
 * control is absent. They are not characterisation tests — when the adapter
 * grows `onEdit`, `canEdit` flips true, the button returns and these fail,
 * which is the correct prompt to replace them with real edit-flow coverage.
 */
import { expect, type Locator, type Page, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;
const USER_ID = 'pw-edit-affordance';
const REPLY = 'canary-edit-affordance-6b4z';

async function resetMock(): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/reset`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({}),
  });
}

async function setMockBehavior(key: string, value: string): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/behavior`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ key, value }),
  });
}

async function openChat(page: Page): Promise<Locator> {
  await bootAuthenticatedPage(page, USER_ID, '/chat');
  await page.goto('/#/chat');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  const input = page.getByTestId('chat-message-input');
  await expect(input).toBeVisible();
  return input;
}

async function waitForSocketConnected(page: Page): Promise<void> {
  await expect
    .poll(
      async () =>
        page.evaluate(() => {
          const store = (
            window as unknown as {
              __OPENHUMAN_STORE__?: {
                getState?: () => { socket?: { byUser?: Record<string, { status?: string }> } };
              };
            }
          ).__OPENHUMAN_STORE__;
          const byUser = store?.getState?.().socket?.byUser ?? {};
          return Object.values(byUser).some(entry => entry?.status === 'connected');
        }),
      { timeout: 30_000 }
    )
    .toBe(true);
}

/** Send one turn so a user message with a hover action bar exists. */
async function sendOneTurn(page: Page, input: Locator, text: string): Promise<Locator> {
  await waitForSocketConnected(page);
  await input.click();
  await page.keyboard.press('ControlOrMeta+a');
  await page.keyboard.press('Delete');
  await page.keyboard.type(text);
  await expect(page.getByTestId('send-message-button')).toBeEnabled();
  await page.getByTestId('send-message-button').click();
  await expect(page.getByText(REPLY).last()).toBeVisible({ timeout: 45_000 });

  const userMessage = page.locator('[data-slot="aui_user-message-root"]').last();
  await expect(userMessage).toBeVisible();
  return userMessage;
}

test.describe('User-message action bar — capability-gated affordances (#5897)', () => {
  test.beforeEach(async () => {
    await resetMock();
    await setMockBehavior('llmForcedResponses', JSON.stringify([{ content: REPLY }]));
  });

  test('no Edit button is offered while the runtime cannot edit', async ({ page }) => {
    const input = await openChat(page);
    const userMessage = await sendOneTurn(page, input, 'a message to hover');

    // Hover is the state in which the action bar reveals its controls, so this
    // is the moment the dead button used to appear.
    await userMessage.hover();
    await page.waitForTimeout(300);

    await expect(page.locator('.aui-user-action-edit')).toHaveCount(0);

    // And no edit composer can be reached, which is the reason the button had
    // to go rather than be left in place.
    await expect(page.locator('.aui-edit-composer-input')).toHaveCount(0);
  });

  test('the action bar itself still renders after the Edit button is withheld', async ({
    page,
  }) => {
    const input = await openChat(page);
    const userMessage = await sendOneTurn(page, input, 'copy me');

    await userMessage.hover();
    await page.waitForTimeout(300);

    // THE CONTROL, and the reason the assertion above is not vacuous: removing
    // the Edit button must not remove the bar it lived in. Without this, a
    // regression that dropped the whole action bar — or never rendered the
    // message — would satisfy "no Edit button" and look like a pass.
    //
    // This asserts the Copy button is PRESENT, not that copying works. The
    // title used to say "Copy still works", which overclaimed — clipboard
    // behaviour is a separate feature this PR does not touch, and testing it
    // here would need clipboard permissions and would not make the control any
    // stronger.
    await expect(page.getByRole('button', { name: 'Copy response' })).toBeVisible();
  });

  test('no branch picker is offered while the runtime cannot switch branches', async ({ page }) => {
    const input = await openChat(page);
    await sendOneTurn(page, input, 'branch check');

    // Same defect class as the Edit button, fixed at the same time: the branch
    // picker was rendered unconditionally at both call sites and was invisible
    // only because assistant-ui's `hideWhenSingleBranch` happened to hold.
    await expect(page.locator('.aui-branch-picker-root')).toHaveCount(0);
  });
});
