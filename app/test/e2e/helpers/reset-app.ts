/**
 * E2E reset — bring the running app back to a fresh-install baseline.
 *
 * We keep ONE Appium session across the whole spec run (see wdio.conf.ts).
 * Instead of restarting the sidecar between specs we call the in-place
 * `openhuman.test_reset` RPC: it wipes the auth marker, clears the
 * `chat_onboarding_completed` flag, removes all cron jobs and saves the
 * config back. The renderer is then reloaded so Redux/localStorage are
 * also flushed, and the spec re-authenticates via the deep-link bypass
 * and walks onboarding through the real UI.
 *
 * Use this as the FIRST thing every spec does:
 *
 *   before(async () => {
 *     await resetApp('e2e-<spec-name>');
 *   });
 *
 * Picking a unique `userId` per spec is what gives each spec its own
 * mock-backend identity so request logs aren't cross-contaminated.
 */
import { waitForApp, waitForAppReady } from './app-helpers';
import { callOpenhumanRpc } from './core-rpc';
import { waitForWebView, waitForWindowVisible } from './element-helpers';
import { triggerAuthLoopbackBypass } from './loopback-auth-helpers';
import { supportsExecuteScript } from './platform';
import { dismissBootCheckGateIfVisible, waitForHomePage, walkOnboarding } from './shared-flows';

interface ResetAppOptions {
  /** Skip the auth + onboarding bootstrap. Use for specs that test the welcome/login screens themselves. */
  skipAuth?: boolean;
  /** Override the onboarding-walker log prefix. */
  logPrefix?: string;
}

function stepLog(message: string): void {
  console.log(`[resetApp][${new Date().toISOString()}] ${message}`);
}

/**
 * Wipe sidecar + renderer state and (by default) re-auth + onboard.
 *
 * Order matters:
 *   1. RPC wipe — must come BEFORE the reload, otherwise the renderer will
 *      re-hydrate Redux from persisted storage and immediately call out to
 *      a sidecar that still thinks the user is logged in.
 *   2. Clear web storage + reload — the renderer's persisted Redux slices
 *      live in localStorage; if we don't drop them the welcome screen is
 *      skipped and we land back on /home with stale state.
 *   3. Wait for the app to remount.
 *   4. Re-auth via deep-link bypass + walk onboarding through the UI.
 *
 * Returns the userId that was authenticated (mirrors what the spec passed
 * in) so callers can use it for mock-backend request-log assertions.
 */
export async function resetApp(userId: string, options: ResetAppOptions = {}): Promise<string> {
  const logPrefix = options.logPrefix ?? '[resetApp]';

  stepLog(`Calling openhuman.test_reset for ${userId}`);
  // The sidecar only spawns after the first successful user login, so the
  // very first spec of a run hits an unreachable RPC — that's not an error,
  // a freshly-launched workspace is already in the same "pristine" state
  // the wipe would have produced. Race the RPC call against a short budget
  // and treat the result as a flag: did we actually wipe anything?
  const reset = await Promise.race([
    callOpenhumanRpc('openhuman.test_reset', {}),
    new Promise<{ ok: false; error: string }>(resolve =>
      setTimeout(
        () =>
          resolve({
            ok: false,
            error: 'test_reset RPC probe timed out (sidecar likely not started)',
          }),
        8_000
      )
    ),
  ]);
  let didWipe = false;
  if (reset.ok) {
    stepLog(`Sidecar wipe ok: ${JSON.stringify(reset.result)}`);
    didWipe = true;

    // test_reset clears onboarding_completed=false (mirrors a fresh install).
    // E2E specs assume an already-onboarded user — restore the flag so
    // App.tsx's onboarding gate doesn't redirect every spec into the wizard.
    const setOnboarding = await callOpenhumanRpc('openhuman.config_set_onboarding_completed', {
      value: true,
    }).catch((err: unknown) => {
      stepLog(`config_set_onboarding_completed failed (non-fatal): ${err}`);
      return { ok: false as const };
    });
    if (setOnboarding.ok) {
      stepLog('Restored onboarding_completed=true after reset');
    }
  } else {
    const errText = String(reset.error ?? '');
    const unreachable =
      errText.includes('not reachable') ||
      errText.includes('probe timed out') ||
      errText.includes('ECONNREFUSED');
    if (!unreachable) {
      throw new Error(`openhuman.test_reset failed: ${errText || JSON.stringify(reset)}`);
    }
    stepLog(`Sidecar not reachable (${errText}) — treating as fresh launch, skipping wipe`);
  }

  // Only reload the renderer when we actually wiped state — otherwise we
  // throw away the in-app "Choose core mode" acceptance the app shell has
  // already cleared, and end up wedged behind that modal on first launch.
  if (didWipe && supportsExecuteScript()) {
    stepLog('Clearing renderer storage + reloading webview');
    await browser.execute(() => {
      try {
        window.localStorage.clear();
        window.sessionStorage.clear();
      } catch (err) {
        console.warn('[resetApp] storage.clear failed', err);
      }
      window.location.replace('#/');
      window.location.reload();
    });
    // window.location.reload() is asynchronous — give the browser time to
    // start the reload before we poll readyState. Without this pause the
    // subsequent waitForApp / waitForAppReady calls may find readyState:
    // 'complete' on the OLD document (before the reload started) and return
    // immediately, racing with the reload and producing a stale auth state.
    await browser.pause(1_000);
  } else if (didWipe) {
    stepLog('execute() unsupported — skipping renderer reload (state may be stale)');
  } else {
    stepLog('Skipping renderer reload — nothing was wiped');
  }

  await waitForApp();
  await waitForWindowVisible(25_000);
  await waitForWebView(15_000);
  await waitForAppReady(15_000);
  await dismissBootCheckGateIfVisible();

  if (options.skipAuth) {
    stepLog('skipAuth=true — waiting for Welcome screen to render');
    // In the shared session, a prior spec may have authenticated and left
    // sessionToken hydrated in the renderer. test_reset + storage.clear +
    // reload normally drops everything, but redux-persist re-hydration can
    // race: the PublicRoute briefly sees a non-empty snapshot and starts
    // navigating to /home before the core's "no active user" snapshot
    // arrives. If that race lands us on /home, the caller's
    // `waitForText('Welcome…')` will fail.
    //
    // Force the renderer to settle on Welcome by polling the route and
    // re-replacing the hash if necessary. Up to 10s; if it still isn't
    // there, surface the issue here instead of in the spec.
    const welcomeDeadline = Date.now() + 10_000;
    let welcomeVisible = false;
    while (Date.now() < welcomeDeadline) {
      welcomeVisible = await browser
        .execute(() => {
          // Welcome.tsx renders an h1 with i18n key welcome.title ('Welcome to OpenHuman').
          const headings = Array.from(document.querySelectorAll('h1'));
          return headings.some(h => /Welcome to OpenHuman/i.test(h.textContent ?? ''));
        })
        .catch(() => false);
      if (welcomeVisible) break;
      // If hash drifted (e.g. to /home), force it back to root so PublicRoute
      // re-renders.
      const hash = await browser.execute(() => window.location.hash).catch(() => '');
      if (typeof hash === 'string' && hash !== '#/' && hash !== '#') {
        await browser
          .execute(() => {
            window.location.replace('#/');
          })
          .catch(() => undefined);
      }
      await browser.pause(300);
    }
    if (welcomeVisible) {
      stepLog('Welcome screen confirmed');
    } else {
      stepLog('Welcome screen still not visible — caller may assert and fail');
    }
    return userId;
  }

  stepLog(`Triggering auth loopback bypass for ${userId}`);
  await triggerAuthLoopbackBypass(userId);
  await waitForAppReady(15_000);
  // BootCheckGate may re-mount after the loopback callback routes to /home;
  // dismiss the modal again if it slid back into view.
  await dismissBootCheckGateIfVisible(8_000);
  await walkOnboarding(logPrefix);

  // Confirm the app actually reached the Home page after auth bypass + onboarding.
  // Without this check, a routing race can leave the renderer stuck at #/ (Welcome)
  // so that every subsequent `navigateViaHash` call is silently redirected back by
  // the auth guard — causing cascading navigation failures in the spec.
  const homeText = await waitForHomePage(15_000).catch(() => null);
  if (!homeText) {
    stepLog('Home page not reached after onboarding — retrying auth bypass');
    await triggerAuthLoopbackBypass(userId);
    await waitForAppReady(10_000);
    await dismissBootCheckGateIfVisible(8_000);
    await walkOnboarding(logPrefix);
    const retryHome = await waitForHomePage(15_000).catch(() => null);
    if (!retryHome) {
      stepLog('Home page still not reached after retry — proceeding anyway');
    } else {
      stepLog(`Home page confirmed on retry: "${retryHome}"`);
    }
  } else {
    stepLog(`Home page confirmed: "${homeText}"`);
  }

  stepLog('Reset + onboarding complete');
  return userId;
}
