import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, callCoreRpc, seedBrowserCoreMode } from '../helpers/core-rpc';

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
 * The vehicle is the MCP `ConnectAuthModal`
 * (`components/channels/mcp/ConnectAuthModal.tsx`): the credential form a
 * declared server's **Connect** button opens. It is the one connector dialog
 * that opens deterministically with no live credentials — the server is
 * declared in `mcp.json` through the core's own RPC with a launcher that never
 * runs — and it is a plain `ModalShell` consumer, so what holds here holds for
 * the others. `mcp-tab-flow.spec.ts` covers what the modal does; this covers
 * only the keyboard and focus surface it does not touch.
 */

const DIALOG = '[role="dialog"]';
const SERVER_NAME = 'pw-focus-trap';

/**
 * Declare the vehicle server through the real core. `enabled` is what makes
 * the detail offer a Connect button; the launcher is a command that exits at
 * once, so the background connect the declaration starts fails harmlessly
 * instead of leaving a subprocess behind.
 *
 * The launcher is the bare name `false`, NOT `/bin/false`. `locate_command`
 * (`tinymcp/.../transport/stdio/spawn_env/mod.rs:296-318`) branches on
 * `has_path_separator`: anything containing `/` is taken as a literal path with
 * no PATH lookup, while a bare name is resolved across PATH. `/bin/false` is a
 * Linux-ism — on macOS `false` lives at `/usr/bin/false` — so the absolute form
 * made the launcher unfindable there, the connect failed with `MissingRuntime`,
 * the server never reached a connected state and the vehicle's row never
 * rendered. Every case in this file then timed out on that row, for a reason CI
 * (Linux, where `/bin/false` exists) never sees. The bare name is correct on
 * both platforms.
 */
async function declareVehicleServer(keyName: string): Promise<void> {
  await callCoreRpc('openhuman.mcp_clients_config_set', {
    mcpServers: { [SERVER_NAME]: { command: 'false', env: { [keyName]: 'seed' } } },
  });
}

async function openConnectDialog(
  page: import('@playwright/test').Page,
  opts: { keyName?: string } = {}
) {
  const keyName = opts.keyName ?? 'NOTION_API_KEY';
  await bootAuthenticatedPage(page, `pw-focus-trap-${keyName.toLowerCase()}`);
  await declareVehicleServer(keyName);
  await seedBrowserCoreMode(page);
  await page.goto('/#/connections?tab=tools');
  await page.waitForSelector('#root');

  // The Servers tab lists the declaration; its detail carries the Connect
  // button that opens the modal.
  const row = page.locator('table tbody tr[data-testid="mcp-installed-row"]', {
    has: page.locator(`td:first-child:has-text("${SERVER_NAME}")`),
  });
  await expect(row).toBeVisible({ timeout: 20_000 });
  const connect = page.getByRole('button', { name: `Connect ${SERVER_NAME}` });
  await expect(connect).toBeVisible({ timeout: 10_000 });

  // A focusable element outside the dialog, so focus restore has somewhere
  // observable to return to. The Connect button itself is the natural
  // trigger, but naming our own anchor keeps the assertion independent of
  // what Radix would restore to on its own.
  await page.evaluate(() => {
    const anchor = document.createElement('button');
    anchor.id = 'pw-focus-anchor';
    anchor.textContent = 'anchor';
    document.body.prepend(anchor);
  });
  await page.locator('#pw-focus-anchor').focus();
  await expect.poll(() => page.evaluate(() => document.activeElement?.id)).toBe('pw-focus-anchor');

  // Open with the keyboard so the anchor, not the Connect button, is what was
  // focused before the dialog took over: `Enter` on the anchor does nothing,
  // so drive the click through the DOM instead of a pointer, which would move
  // focus first.
  await connect.evaluate(button => (button as HTMLButtonElement).click());

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

test.afterEach(async () => {
  // Leave the core as it was found: the vehicle is a throwaway declaration.
  await callCoreRpc('openhuman.mcp_clients_config_set', { mcpServers: {} }).catch(() => {});
});

// A fourth test ("the element behind the dialog cannot be reached by keyboard",
// asserting `document.activeElement.id !== 'pw-focus-anchor'` after 12 Tabs) was
// written and then REMOVED: with the focus trap deliberately deleted it still
// passed, because focus escaping the dialog does not necessarily land on that
// one element. It was strictly weaker than the Tab test below, which fails on
// the third press. Do not re-add it.
// TODO(#6398): restore isolated core lifecycle coverage for this browser suite.
//
// STILL QUARANTINED, deliberately. The portability defect above is fixed, which
// unblocks reproducing this locally on macOS, but it is NOT the reported
// failure: the issue describes repeated ECONNREFUSED in CI when the core at
// 127.0.0.1:17788 disappears mid-suite, and that has not been reproduced or
// explained. A run on macOS at `0f1ecc9d2` found the core healthy throughout —
// 414 log lines, no panic or abort, every `mcp_clients_config_set` returning
// ok, and the log ending in the session's own SIGTERM teardown — so the
// premise in the title ("its test core exits") is not established.
//
// Un-skipping now would remove the marker without fixing the behaviour it was
// raised for, which is worse than leaving it: the suite would be restored on a
// guess, and because `.github/workflows/e2e-playwright.yml` is
// `on: workflow_dispatch: {}` with no push or pull_request trigger, nothing
// would contradict the guess automatically.
//
// What restoring this needs, in order: (1) a macOS run, which additionally
// needs #6476 — `e2e-web-session.sh` launches the core through `setsid`, which
// is util-linux and absent on macOS, so the core never starts; (2) a CI run
// that reproduces the ECONNREFUSED with `core.log` and `core-resource.log`
// retained (the workflow uploads both on failure, 7-day retention) to separate
// a runner OOM from an in-process failure.
//
// Worth knowing before deleting instead of restoring: `mcp-tab-flow.spec.ts`
// covers the same route and selector but is entirely mocked — it installs
// `page.route('**/rpc', ...)` and fulfils the RPCs itself, with zero
// `callCoreRpc`. This suite is therefore the only spec exercising this surface
// against a live core.
test.describe('Connector modal — focus containment', () => {
  // Precondition assertion, not mutation-proven: this survived BOTH the
  // trap-deletion mutation and removing the input's `autoFocus`, because Radix
  // moves focus in on open independently of either. Kept because a dialog that
  // opens without taking focus leaves a keyboard user typing into the page
  // behind it — but do not count it as a guard against the trap regressing;
  // the Tab test below is that guard.
  test('moves focus into the dialog when it opens', async ({ page }) => {
    await openConnectDialog(page);
    expect(await focusIsInsideDialog(page)).toBe(true);
  });

  test('Tab cycles within the dialog and never escapes to the page behind', async ({ page }) => {
    await openConnectDialog(page);

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
    await openConnectDialog(page);

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
    const dialog = await openConnectDialog(page);
    await page.keyboard.press('Escape');
    await expect(dialog).not.toBeVisible({ timeout: 10_000 });
  });

  test('Escape restores focus to whatever was focused before it opened', async ({ page }) => {
    // `ModalShell` does this itself rather than leaving it to Radix
    // (:57-60: Radix restores to the trigger, and a dialog opened without a
    // pointer on its trigger would otherwise drop focus to <body>).
    const dialog = await openConnectDialog(page);
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
  test('Escape does not submit the credential', async ({ page }) => {
    // Dismissing must be a cancel, not an accidental send of a typed token.
    const submitted: string[] = [];
    await page.route('**/rpc', async (route, request) => {
      const body = JSON.parse(request.postData() || '{}');
      if (
        body.method === 'openhuman.mcp_clients_update_env' ||
        body.method === 'openhuman.mcp_clients_connect'
      ) {
        submitted.push(String(body.method));
      }
      await route.continue();
    });

    const dialog = await openConnectDialog(page);
    await dialog.locator('input[type="password"]').first().fill('ntn_should_never_be_sent');
    await page.keyboard.press('Escape');
    await expect(dialog).not.toBeVisible({ timeout: 10_000 });

    expect(submitted).toEqual([]);
  });
});

test.describe('Connector modal — accessible shape', () => {
  test('is a labelled modal dialog, not a bare overlay', async ({ page }) => {
    const dialog = await openConnectDialog(page);
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

  test('names the key being asked for so the user knows what they are pasting', async ({
    page,
  }) => {
    const dialog = await openConnectDialog(page, { keyName: 'LINEAR_API_KEY' });
    await expect(dialog.locator('label[for="auth-LINEAR_API_KEY"]')).toContainText(
      'LINEAR_API_KEY'
    );
  });

  test('keeps the credential field masked', async ({ page }) => {
    const dialog = await openConnectDialog(page);
    await expect(dialog.locator('input[type="password"]').first()).toBeVisible();
  });
});
