import { expect, test } from '@playwright/test';

import { bootRuntimeReadyGuestPage } from '../helpers/core-rpc';

/**
 * Modal keyboard and focus behaviour, driven in a real browser.
 *
 * Every connector setup flow on `/connections` renders through `ModalShell`
 * (`components/ui/ModalShell.tsx`), which wraps Radix for the focus trap and
 * adds its own focus restore. Its module doc names the bug it was written for:
 * "there was none — Tab escaped the dialog into the page behind it" (:48).
 *
 * **None of that is testable in jsdom.** jsdom has no layout, no real focus
 * ring, and does not implement sequential focus navigation, so `Tab` moves
 * nothing — a jsdom test asserting a focus trap passes whether or not the trap
 * exists. This spec is the only place that behaviour is actually exercised.
 *
 * The vehicle is `SecretPromptDialog` (`components/mcp-setup/SecretPromptDialog.tsx`),
 * opened by dispatching the `openhuman:mcp-setup-secret-requested` event the
 * socket bridge normally publishes. It is the one connector dialog that opens
 * deterministically with no live credentials, and it is a plain `ModalShell`
 * consumer, so what holds here holds for the others.
 * `mcp-setup-secret-flow.spec.ts` already covers submit / cancel / show-hide;
 * this covers only the keyboard and focus surface it does not touch.
 */

const DIALOG = '[role="dialog"]';

async function openSecretDialog(
  page: import('@playwright/test').Page,
  opts: { keyName?: string } = {}
) {
  await bootRuntimeReadyGuestPage(page);

  // A focusable element outside the dialog, so focus restore has somewhere
  // observable to return to.
  await page.evaluate(() => {
    const anchor = document.createElement('button');
    anchor.id = 'pw-focus-anchor';
    anchor.textContent = 'anchor';
    document.body.prepend(anchor);
    anchor.focus();
  });
  await expect.poll(() => page.evaluate(() => document.activeElement?.id)).toBe('pw-focus-anchor');

  await page.evaluate(
    ({ keyName }) => {
      window.dispatchEvent(
        new CustomEvent('openhuman:mcp-setup-secret-requested', {
          detail: {
            refId: 'secret://pwfocus0001',
            keyName,
            prompt: 'Enter your integration token to connect.',
          },
        })
      );
    },
    { keyName: opts.keyName ?? 'NOTION_API_KEY' }
  );

  const dialog = page.locator(DIALOG);
  await expect(dialog).toBeVisible({ timeout: 10_000 });
  return dialog;
}

/** Is the currently focused element inside the dialog? */
const focusIsInsideDialog = (page: import('@playwright/test').Page) =>
  page.evaluate(() => {
    const dialog = document.querySelector('[role="dialog"]');
    const active = document.activeElement;
    return Boolean(dialog && active && dialog.contains(active));
  });

// A fourth test ("the element behind the dialog cannot be reached by keyboard",
// asserting `document.activeElement.id !== 'pw-focus-anchor'` after 12 Tabs) was
// written and then REMOVED: with the focus trap deliberately deleted it still
// passed, because focus escaping the dialog does not necessarily land on that
// one element. It was strictly weaker than the Tab test below, which fails on
// the third press. Do not re-add it.
test.describe('Connector modal — focus containment', () => {
  // Precondition assertion, not mutation-proven: this survived BOTH the
  // trap-deletion mutation and removing the input's `autoFocus`, because Radix
  // moves focus in on open independently of either. Kept because a dialog that
  // opens without taking focus leaves a keyboard user typing into the page
  // behind it — but do not count it as a guard against the trap regressing;
  // the Tab test below is that guard.
  test('moves focus into the dialog when it opens', async ({ page }) => {
    await openSecretDialog(page);
    expect(await focusIsInsideDialog(page)).toBe(true);
  });

  test('Tab cycles within the dialog and never escapes to the page behind', async ({ page }) => {
    await openSecretDialog(page);

    // Walk further than the dialog has focusables, so an unbounded sequence
    // would certainly have left it.
    for (let i = 0; i < 12; i++) {
      await page.keyboard.press('Tab');
      expect(
        await focusIsInsideDialog(page),
        `focus left the dialog after ${i + 1} Tab press(es)`
      ).toBe(true);
    }
  });

  test('Shift+Tab is contained too', async ({ page }) => {
    await openSecretDialog(page);

    for (let i = 0; i < 12; i++) {
      await page.keyboard.press('Shift+Tab');
      expect(
        await focusIsInsideDialog(page),
        `focus left the dialog after ${i + 1} Shift+Tab press(es)`
      ).toBe(true);
    }
  });
});

test.describe('Connector modal — Escape', () => {
  test('Escape closes the dialog', async ({ page }) => {
    const dialog = await openSecretDialog(page);
    await page.keyboard.press('Escape');
    await expect(dialog).not.toBeVisible({ timeout: 10_000 });
  });

  test('Escape restores focus to whatever was focused before it opened', async ({ page }) => {
    // `ModalShell` does this itself rather than leaving it to Radix
    // (:57-60: Radix restores to the trigger, and a dialog opened by an event
    // has no trigger, so focus would drop to <body>).
    const dialog = await openSecretDialog(page);
    await page.keyboard.press('Escape');
    await expect(dialog).not.toBeVisible({ timeout: 10_000 });

    await expect
      .poll(() => page.evaluate(() => document.activeElement?.id), { timeout: 10_000 })
      .toBe('pw-focus-anchor');
  });

  // Negative guard: no realistic single-line mutation makes Escape submit, so
  // this is not mutation-proven. It is here because the failure it guards
  // against — a dismissed dialog quietly sending a typed token — is severe
  // enough to be worth a standing assertion.
  test('Escape does not submit the secret', async ({ page }) => {
    // Dismissing must be a cancel, not an accidental send of a typed token.
    const submitted: string[] = [];
    await page.route('**/rpc', async (route, request) => {
      const body = JSON.parse(request.postData() || '{}');
      if (body.method === 'openhuman.mcp_setup_submit_secret') {
        submitted.push(String(body.params?.ref_id ?? ''));
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ jsonrpc: '2.0', id: body.id, result: { fulfilled: true } }),
        });
        return;
      }
      await route.continue();
    });

    const dialog = await openSecretDialog(page);
    await dialog.locator('input[type="password"]').fill('ntn_should_never_be_sent');
    await page.keyboard.press('Escape');
    await expect(dialog).not.toBeVisible({ timeout: 10_000 });

    expect(submitted).toEqual([]);
  });
});

test.describe('Connector modal — accessible shape', () => {
  test('is a labelled modal dialog, not a bare overlay', async ({ page }) => {
    const dialog = await openSecretDialog(page);
    // A screen reader needs both of these to announce it as a dialog and read
    // its name; `ModalShell` wires `aria-labelledby` from its title (:100).
    await expect(dialog).toHaveAttribute('aria-labelledby', /.+/);
    const labelledBy = await dialog.getAttribute('aria-labelledby');
    const labelText = await page.evaluate(
      id => document.getElementById(id ?? '')?.textContent ?? '',
      labelledBy
    );
    expect(labelText.trim().length).toBeGreaterThan(0);
  });

  test('names the key being requested so the user knows what they are pasting', async ({
    page,
  }) => {
    const dialog = await openSecretDialog(page, { keyName: 'LINEAR_API_KEY' });
    await expect(dialog.locator('code')).toContainText('LINEAR_API_KEY');
  });

  test('keeps the secret field masked', async ({ page }) => {
    const dialog = await openSecretDialog(page);
    await expect(dialog.locator('input[type="password"]')).toBeVisible();
  });
});
