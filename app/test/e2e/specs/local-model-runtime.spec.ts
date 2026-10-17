// @ts-nocheck
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { triggerAuthDeepLink } from '../helpers/deep-link-helpers';
import {
  textExists,
  visibleTextExists,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import {
  dismissWalkthroughIfVisible,
  waitForHomePage,
  walkOnboarding,
} from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

/**
 * Local model runtime — the app-managed surface is gone; the route redirects.
 *
 * # What this file used to be, and why it was replaced
 *
 * Until 2026-09-24 this spec was a single `describe.skip`ped case driving a
 * "Local model runtime" card, a "Manage" button and a "Runtime Status" panel,
 * with the reason given as *"CI does not provision a live Ollama server, so
 * keep this spec skipped until a deterministic mockable local-runtime harness
 * exists for WDIO."*
 *
 * That reason had stopped being the real one. Commit `0ec68613af` removed the
 * local-model debug panel outright, so every control the old case reached no
 * longer exists — unskipping it would not have needed an Ollama server, it
 * would have failed on the first `waitForText('Local model runtime')`. The
 * spec was not waiting for a harness; it was testing deleted UI.
 *
 * That mattered beyond tidiness: `docs/TEST-COVERAGE-MATRIX.md` cited this file
 * as the evidence for **3.1.1, 3.1.2, 3.2.1, 3.2.3** (all ✅) and **3.3.3.2**
 * (🟡). Five rows rested on a spec that had never executed an assertion, and
 * because the file *is* wired into the `system` suite it was collected,
 * launched and reported as a pass every run. A skipped spec that four ✅ rows
 * point at is worse than no spec: it reads as coverage from the matrix and
 * costs a lane slot to produce nothing.
 *
 * # What this file asserts now
 *
 * The one claim about the local-model surface that is true, checkable on the
 * desktop shell, and worth defending: **the legacy deep route is a redirect,
 * not a dead end.** `settingsRouteElements.tsx` maps `local-model-debug` to
 * `<Navigate to="/connections?tab=llm" replace />`, so a user following an old
 * link, bookmark or doc lands on the page that replaced it. If that `Navigate`
 * is dropped in a future settings-route refactor the route renders nothing and
 * the failure is silent — a blank settings pane, no console error.
 *
 * The web lane asserts the same redirect
 * (`app/test/playwright/specs/local-model-runtime.spec.ts`); this is the
 * desktop-shell half, where the hash router runs inside the real Wry webview
 * rather than a browser tab.
 *
 * `navigateViaHash` is deliberately not used: its `HASH_REDIRECTS` table does
 * not carry this route, so it would wait for the wrong target hash and time
 * out. Driving `window.location.hash` and polling for the settled value tests
 * the app's own router rather than the helper's copy of the route map — which
 * is the point, since a stale copy is exactly what this guards against.
 */
describe('Local model runtime route', () => {
  before(async function beforeSuite() {
    this.timeout(90_000);
    await startMockServer();
    await waitForApp();
  });

  after(async () => {
    await stopMockServer();
  });

  it('redirects the retired local-model-debug route to the Connections LLM tab', async function () {
    this.timeout(90_000);

    await triggerAuthDeepLink('e2e-local-model-token');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await walkOnboarding('[LocalModel]');
    await waitForHomePage(20_000);
    await dismissWalkthroughIfVisible(20_000);
    await browser.waitUntil(async () => !(await visibleTextExists('Skip tour')), {
      timeout: 3_000,
      interval: 250,
      timeoutMsg: 'walkthrough must be dismissed before route navigation',
    });

    await browser.execute(() => {
      window.location.hash = '#/settings/local-model-debug';
    });

    let settled = '';
    await browser.waitUntil(
      async () => {
        settled = await browser.execute(() => window.location.hash);
        // Wait for the redirect to land, not merely for the hash to change:
        // the requested route is itself a valid intermediate value.
        return typeof settled === 'string' && settled.includes('/connections');
      },
      {
        timeout: 20_000,
        interval: 300,
        timeoutMsg:
          'requesting #/settings/local-model-debug should have redirected to /connections; ' +
          'if the Navigate in settingsRouteElements.tsx was dropped the route renders nothing',
      }
    );

    expect(settled).toContain('/connections');
    expect(settled).not.toContain('local-model-debug');

    // The removed panel must not come back through a different route: its
    // distinctive controls are the ones the pre-0ec68613af spec drove.
    expect(await textExists('Runtime Status')).toBe(false);
  });
});
