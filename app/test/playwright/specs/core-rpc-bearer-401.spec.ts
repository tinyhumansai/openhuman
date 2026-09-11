import { expect, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

/**
 * A 401 from the LOCAL core's RPC bearer gate must not sign the user out
 * (PR #5876).
 *
 * `getCoreRpcToken` caches the resolved bearer for the lifetime of the frontend
 * process, so an in-process core restart — which mints a fresh per-launch token
 * — leaves the renderer holding a stale one and every subsequent RPC 401s.
 * Before #5876 `classifyRpcError` mapped any 401 to `auth_expired`, and
 * `classifyAuthExpiredReason` paired it with `confirmed`, which skips
 * corroboration in `CoreStateProvider` and calls `clearSession()` — wiping the
 * TinyHumans auth profile from disk because the *local* core rejected a bearer.
 * The TinyHumans server had said nothing at all.
 *
 * #5876 introduced a distinct `core_auth` kind for exactly this case: it does
 * not dispatch auth-expired, and it drops the token cache and retries once with
 * a freshly-read bearer.
 *
 * Note on what this does NOT cover, deliberately: a 401 from the *backend*
 * (a genuinely revoked session) must still sign the user out. That is the
 * complementary case and `app/test/e2e/specs/auth-access-control.spec.ts`
 * ("Revoked session auto-logout") owns it. The two must not be conflated — the
 * whole point of #5876 is that they are different routes with opposite
 * handling.
 */

/** The RPC the embeddings tab issues on open — a deterministic trigger. */
const TRIGGER_METHOD = 'openhuman.embeddings_get_settings';

test.describe('Core RPC bearer 401 — recovery, not logout', () => {
  test('retries once with a fresh bearer and keeps the user signed in', async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-core-401', '/connections');
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);

    // Fault injection modelled on a real core restart: the FIRST call to the
    // trigger method is rejected AND the stored bearer is rotated, exactly as a
    // core that restarted and minted a fresh per-launch token would leave
    // things. The retry must therefore present a DIFFERENT bearer to succeed —
    // counting requests alone would let a retry that reused the stale cached
    // token pass, which is the very regression #5876 guards against.
    const ROTATED = 'openhuman-playwright-token-rotated';
    const seenAuth: string[] = [];
    const excessAttempts: string[] = [];
    await page.route('**/rpc', async (route, request) => {
      let body: { method?: string } = {};
      try {
        body = JSON.parse(request.postData() || '{}');
      } catch {
        /* not JSON — pass through */
      }
      if (body.method === TRIGGER_METHOD) {
        seenAuth.push(request.headers()['authorization'] ?? '');
        if (seenAuth.length > 2) {
          // Recorded HERE, not asserted after the poll: `seenAuth.length` grows
          // when the handler STARTS, and `route.fallback()` settles
          // asynchronously, so a third request can arrive after a mid-flight
          // `<= 2` check and escape it entirely. The handler is the only place
          // that observes every attempt.
          excessAttempts.push(request.headers()['authorization'] ?? '');
        }
        if (seenAuth.length === 1) {
          // Rotate the stored bearer before rejecting, so a refreshed read
          // picks up the new value and a cached read does not.
          await page.evaluate(
            token => window.localStorage.setItem('openhuman_core_rpc_token', token),
            ROTATED
          );
          return route.fulfill({
            status: 401,
            contentType: 'text/plain',
            body: 'Missing or invalid Authorization header',
          });
        }
      }
      return route.fallback();
    });

    // Trigger it.
    await page.evaluate(() => {
      window.location.hash = '/connections?tab=embeddings';
    });

    // (1) The retry happened at all. Without #5876 the 401 classifies as
    // `auth_expired`, nothing retries, and this stays at 1.
    await expect.poll(() => seenAuth.length, { timeout: 20_000 }).toBeGreaterThanOrEqual(2);

    // (2) And it used a FRESH bearer — the assertion that distinguishes a real
    // recovery from a blind re-send. `clearCoreRpcTokenCache()` is what makes
    // the second read see the rotated value.
    expect(seenAuth[0]).toContain('openhuman-playwright-token');
    expect(seenAuth[1]).toContain(ROTATED);
    expect(seenAuth[1]).not.toEqual(seenAuth[0]);

    // (3) The session survives. Without #5876 `clearSession()` wipes the auth
    // profile and the app falls back to the signed-out surface. Assert the
    // embeddings panel actually rendered rather than merely that the hash is
    // unchanged — the page was already on /connections before the fault.
    await expect(page.getByTestId('two-pane-nav-embeddings')).toHaveAttribute(
      'aria-current',
      'page',
      { timeout: 20_000 }
    );

    // (4) Bounded to ONE extra attempt. Asserted only now, after the flow has
    // settled into its terminal rendered state, and from the handler-side
    // record so a late third request cannot slip past a mid-flight check. A
    // retry loop against a core that is genuinely rejecting us would be worse
    // than the bug #5876 fixed.
    expect(excessAttempts).toEqual([]);
    expect(seenAuth.length).toBe(2);
  });
});
