import { expect, test } from '@playwright/test';

import { bootRuntimeReadyGuestPage, callCoreRpc } from '../helpers/core-rpc';

// TS-E2E S-06: a signed-out renderer has no socket, so it must find the
// keyring prompt a first sign-in raises by polling
// `keyring_consent_list_pending_operations`, show it, and let the user allow
// it. The lane's test responder answers these prompts by default, so this
// spec runs only on a core started with OPENHUMAN_TEST_KEYRING_CONSENT=off.

const USER_ID = 'ts-e2e-s06-user';
const RESPONDER_ON = process.env.OPENHUMAN_TEST_KEYRING_CONSENT !== 'off';

function sessionJwt(userId: string): string {
  const payload = Buffer.from(
    JSON.stringify({ sub: userId, userId, exp: Math.floor(Date.now() / 1000) + 3600 })
  ).toString('base64url');
  return `eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.${payload}.sig`;
}

test.describe('keyring consent before sign-in', () => {
  const reason = 'S-06 needs the keyring responder off';
  test.skip(RESPONDER_ON, `${reason}: https://github.com/tinyhumansai/openhuman/issues/8598`);

  test('s_06 shows the sign-in keyring prompt without a socket and signs in once allowed', async ({
    page,
  }) => {
    test.setTimeout(180_000);
    await bootRuntimeReadyGuestPage(page);

    const token = sessionJwt(USER_ID);
    let settled = false;
    let outcome = 'pending';
    const signIn = callCoreRpc('openhuman.auth_set_credential', {
      token,
      kind: 'session',
      userId: USER_ID,
      user: { _id: USER_ID, id: USER_ID, displayName: 'TS-E2E S-06' },
    })
      .then(
        () => {
          outcome = 'ok';
        },
        (error: unknown) => {
          outcome = `error: ${String(error)}`;
        }
      )
      .finally(() => {
        settled = true;
      });

    const dialog = page.getByTestId('keyring-operation-approval-dialog');
    await expect(dialog)
      .toBeVisible({ timeout: 20_000 })
      .catch(async (error: unknown) => {
        // Tell "the core raised no prompt" apart from "the renderer missed it".
        const pending = await callCoreRpc('openhuman.keyring_consent_list_pending_operations', {});
        throw new Error(
          `${String(error)}\nsign-in outcome: ${outcome}\ncore pending operations: ${JSON.stringify(pending)}`
        );
      });
    await expect(dialog).toContainText('openhuman.credentials');
    await expect(dialog).not.toContainText(token);

    const deadline = Date.now() + 120_000;
    let allowed = 0;
    while (!settled && Date.now() < deadline) {
      if (await dialog.isVisible()) {
        // The next prompt can replace this one mid-click; a missed click retries.
        const clicked = await dialog
          .getByRole('button', { name: 'Allow once' })
          .click({ timeout: 2_000 })
          .then(() => true)
          .catch(() => false);
        if (clicked) allowed += 1;
      }
      await page.waitForTimeout(250);
    }

    await signIn;
    expect(outcome).toBe('ok');
    expect(allowed).toBeGreaterThan(0);
    const state = await callCoreRpc<{
      isAuthenticated?: boolean;
      result?: { isAuthenticated?: boolean };
    }>('openhuman.auth_get_state');
    expect(state.result?.isAuthenticated ?? state.isAuthenticated).toBe(true);
  });
});
