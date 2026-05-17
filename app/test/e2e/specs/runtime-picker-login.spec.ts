// @ts-nocheck
/**
 * E2E test: Runtime picker → provider login → onboarding/home → logout.
 *
 * Exercises the *first-launch login funnel* end-to-end against the shared
 * mock backend, running on the unified Appium chromium-driver session (CEF
 * over CDP) — the same harness CI uses for Linux in `e2e/docker-compose.yml`.
 *
 *   Phase 1 — Runtime picker (BootCheckGate ModePicker):
 *     1. Reset to Welcome (no auth), then click "Select a Runtime" so
 *        Welcome dispatches `resetCoreMode()` and the BootCheckGate
 *        re-renders the picker.
 *     2. Verify both runtime options ("Run Locally", "Run on the Cloud")
 *        plus the picker heading are present.
 *     3. Cloud branch:
 *          - URL/token inputs appear when cloud is selected.
 *          - Empty URL on Continue → inline URL error.
 *          - Empty token → inline token error.
 *          - Unreachable host on "Test Connection" → unreachable status pill
 *            (no auth backend at 127.0.0.1:1 / picks a closed port).
 *     4. Switch back to Local, click Continue. BootCheckGate runs `runBootCheck`
 *        against the embedded local core (which is already up — the e2e build
 *        seeds `VITE_OPENHUMAN_E2E_DEFAULT_CORE_MODE=local`) and we land back
 *        on Welcome with the OAuth provider row visible.
 *
 *   Phase 2 — Provider login (deep-link bypass simulates the OAuth round-trip):
 *     5. Welcome shows OAuth provider buttons. We don't click them (that opens
 *        the system browser), instead we simulate the post-OAuth deep link
 *        callback — exactly the same code path the real providers exercise
 *        when the backend redirects back to `openhuman://auth?token=...&key=auth`.
 *     6. Walk onboarding (if shown) until we reach Home.
 *     7. Verify mock backend recorded the auth/me profile fetch.
 *
 *   Phase 3 — Logout:
 *     8. Logout from Settings.
 *     9. Confirm we're back on Welcome (logged-out state visible).
 *
 * The mock server (scripts/mock-api-*) handles auth + profile + onboarding.
 * Deep links go through `window.__simulateDeepLink` so the spec is safe on
 * the headless Linux container — no system browser, no real OAuth round-trip,
 * and no PID-bound URL handler is touched.
 */
import { waitForApp, waitForAppReady, waitForAuthBootstrap } from '../helpers/app-helpers';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  dumpAccessibilityTree,
  hasAppChrome,
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { supportsExecuteScript } from '../helpers/platform';
import { resetApp } from '../helpers/reset-app';
import {
  logoutViaSettings,
  waitForHomePage,
  waitForLoggedOutState,
  waitForRequest,
  walkOnboarding,
} from '../helpers/shared-flows';
import {
  clearRequestLog,
  getRequestLog,
  resetMockBehavior,
  setMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

const LOG = '[RuntimePicker]';

/**
 * Click the smallest clickable element whose textContent contains `text`.
 *
 * Picker option tiles have a title + description nested in a single button so
 * the button's textContent is `<title><description>` — strict equality misses.
 * We score by descendant count to prefer the most-specific match (e.g. the
 * Continue button text "Continue" matches several ancestors; we want the
 * <button> itself, not <body>).
 */
async function clickByTextDom(text: string): Promise<boolean> {
  if (!supportsExecuteScript()) return false;
  return browser.execute(t => {
    const all = Array.from(document.querySelectorAll<HTMLElement>('button, [role="button"], a'));
    const matches = all.filter(el => (el.textContent ?? '').includes(t));
    if (matches.length === 0) return false;
    matches.sort((a, b) => a.querySelectorAll('*').length - b.querySelectorAll('*').length);
    const clickable = matches[0];
    ['mousedown', 'mouseup', 'click'].forEach(type => {
      clickable.dispatchEvent(
        new MouseEvent(type, { bubbles: true, cancelable: true, view: window, button: 0 })
      );
    });
    return true;
  }, text);
}

/** Set value of a controlled input by selector + dispatch React change. */
async function fillInput(selector: string, value: string): Promise<boolean> {
  if (!supportsExecuteScript()) return false;
  return browser.execute(
    ({ sel, val }) => {
      const input = document.querySelector(sel) as HTMLInputElement | null;
      if (!input) return false;
      const setter = Object.getOwnPropertyDescriptor(
        window.HTMLInputElement.prototype,
        'value'
      )?.set;
      setter?.call(input, val);
      input.dispatchEvent(new Event('input', { bubbles: true }));
      input.dispatchEvent(new Event('change', { bubbles: true }));
      return true;
    },
    { sel: selector, val: value }
  );
}

/** Open the BootCheckGate ModePicker by clicking Welcome's "Select a Runtime". */
async function openRuntimePicker(): Promise<boolean> {
  // The button text comes from `welcome.selectRuntime` = "Select a Runtime".
  // We can't disambiguate from the picker heading by text alone, so we trigger
  // through Welcome's button and then assert *picker-only* markers (the option
  // tiles) to confirm we landed on the picker phase.
  const clicked = await clickByTextDom('Select a Runtime');
  if (!clicked) return false;
  await browser.pause(1_000);
  return Boolean(
    (await waitForText('Run Locally (Recommended)', 8_000).catch(() => false)) ||
    (await textExists('Run on the Cloud (Complex)'))
  );
}

describe('Runtime picker → login → onboarding → home → logout', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    resetMockBehavior();
    setMockBehavior('composioConnections', '[]');
    // skipAuth so we land on Welcome (logged out) — the spec drives login itself.
    await resetApp('e2e-runtime-picker-login', { skipAuth: true });
    clearRequestLog();
  });

  after(async () => {
    resetMockBehavior();
    await stopMockServer();
  });

  // -------------------------------------------------------------------------
  // Phase 1: Runtime picker
  // -------------------------------------------------------------------------

  it('app is running and shows Welcome with OAuth providers', async () => {
    expect(await hasAppChrome()).toBe(true);
    await waitForWindowVisible(20_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);

    // Welcome.tsx: "Welcome to OpenHuman" + at least one provider button.
    expect(await waitForText('Welcome to OpenHuman', 15_000)).toBeTruthy();
    expect(await textExists('Select a Runtime')).toBe(true);
  });

  it('clicking "Select a Runtime" opens the runtime picker with both options', async () => {
    const opened = await openRuntimePicker();
    if (!opened) {
      const tree = await dumpAccessibilityTree();
      console.log(`${LOG} Picker did not open. Tree:\n`, tree.slice(0, 4000));
    }
    expect(opened).toBe(true);

    expect(await textExists('Run Locally (Recommended)')).toBe(true);
    expect(await textExists('Run on the Cloud (Complex)')).toBe(true);
  });

  it('cloud option reveals URL + token inputs and validates them', async () => {
    // Click the cloud tile.
    const clickedCloud = await clickByTextDom('Run on the Cloud (Complex)');
    expect(clickedCloud).toBe(true);
    await browser.pause(500);

    // URL field shows up.
    expect(await textExists('Runtime URL')).toBe(true);
    expect(await textExists('Auth Token')).toBe(true);

    // Continue with empty URL → URL error inline.
    const continueClicked = await clickByTextDom('Continue');
    expect(continueClicked).toBe(true);
    await browser.pause(500);
    expect(await textExists('Please enter a runtime URL.')).toBe(true);

    // Fill URL but leave token empty → token error.
    const urlOk = await fillInput('input[type="url"]', 'http://127.0.0.1:1/rpc');
    expect(urlOk).toBe(true);
    await clickByTextDom('Continue');
    await browser.pause(500);
    expect(await textExists("We'll need an auth token to connect.")).toBe(true);
  });

  it('"Test Connection" against an unreachable host shows the unreachable pill', async () => {
    // Token already required; supply something + a deliberately closed port.
    const tokenOk = await fillInput('input[type="password"]', 'bad-token-e2e');
    expect(tokenOk).toBe(true);

    const clicked = await clickByTextDom('Test Connection');
    expect(clicked).toBe(true);

    // Either "auth failed" (if something happens to respond) or unreachable.
    // Both prove the test path actually fired. Poll up to 20s — chromium-driver
    // can sit on the connect timeout for a while before failing.
    const deadline = Date.now() + 20_000;
    let saw = false;
    while (Date.now() < deadline) {
      if (
        (await textExists("Couldn't reach it:")) ||
        (await textExists("That token didn't work. Double-check it and try again."))
      ) {
        saw = true;
        break;
      }
      await browser.pause(500);
    }
    if (!saw) {
      const tree = await dumpAccessibilityTree();
      console.log(`${LOG} No test-connection result. Tree:\n`, tree.slice(0, 4000));
    }
    expect(saw).toBe(true);
  });

  it('switching back to Local and clicking Continue closes the picker', async () => {
    expect(await clickByTextDom('Run Locally (Recommended)')).toBe(true);
    await browser.pause(500);
    expect(await clickByTextDom('Continue')).toBe(true);

    // BootCheckGate flips to 'checking' then 'match' against the in-process
    // local core. Eventually we either land on Welcome (still logged out) or
    // — if onboarding state leaked — on the onboarding overlay. Either is a
    // valid post-picker state; we only care that the picker is gone.
    const deadline = Date.now() + 25_000;
    let pickerGone = false;
    while (Date.now() < deadline) {
      const stillThere =
        (await textExists('Run Locally (Recommended)')) ||
        (await textExists('Run on the Cloud (Complex)'));
      if (!stillThere) {
        pickerGone = true;
        break;
      }
      await browser.pause(500);
    }
    if (!pickerGone) {
      const tree = await dumpAccessibilityTree();
      console.log(`${LOG} Picker did not dismiss. Tree:\n`, tree.slice(0, 4000));
    }
    expect(pickerGone).toBe(true);

    // We should be back on Welcome (logged-out marker).
    const back = await waitForLoggedOutState(15_000);
    expect(back).not.toBeNull();
  });

  // -------------------------------------------------------------------------
  // Phase 2: Provider login (bypass deep link simulates the OAuth callback)
  // -------------------------------------------------------------------------

  it('OAuth provider buttons render on Welcome', async () => {
    // Real OAuth opens a system browser — out of scope for headless CI. We
    // just assert the buttons mount; the deep-link callback below covers the
    // post-OAuth path.
    const providerButtonPresent = await browser.execute(() => {
      const buttons = Array.from(document.querySelectorAll('button'));
      return buttons.some(b => {
        const label = b.getAttribute('aria-label') || b.textContent || '';
        return /Google|GitHub|Twitter|Discord/i.test(label);
      });
    });
    expect(providerButtonPresent).toBe(true);
  });

  it('deep-link auth callback signs the user in and reaches Home', async () => {
    clearRequestLog();
    await triggerAuthDeepLinkBypass('e2e-runtime-picker-user');
    await waitForWindowVisible(20_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await waitForAuthBootstrap(20_000);

    // The bypass path does not call the token-consume endpoint (it sets the
    // JWT directly) — that's by design. The /auth/me lookup MUST still fire.
    const meCall = await waitForRequest(getRequestLog, 'GET', '/auth/me', 20_000);
    if (!meCall) {
      console.log(`${LOG} /auth/me not seen. Log:`, JSON.stringify(getRequestLog(), null, 2));
    }
    expect(meCall).toBeTruthy();

    // Walk through onboarding if it's shown (new user path); a returning user
    // would skip directly to Home. walkOnboarding is a no-op when there's no
    // onboarding-next-button mounted.
    await walkOnboarding(LOG);

    // Confirm we're authenticated + post-onboarding. waitForHomePage's
    // hardcoded greeting strings (Good morning / Test / etc.) can miss
    // valid Home renders, so fall back to a route + welcome-gone check.
    const home = await waitForHomePage(15_000);
    if (home) {
      console.log(`${LOG} Home reached: "${home}"`);
    } else {
      const deadline = Date.now() + 15_000;
      let onHome = false;
      while (Date.now() < deadline) {
        const hash = (await browser.execute(() => window.location.hash)) as string;
        const stillOnWelcome = await textExists('Welcome to OpenHuman');
        if (!stillOnWelcome && (hash.startsWith('#/home') || hash.startsWith('#/chat'))) {
          onHome = true;
          break;
        }
        await browser.pause(500);
      }
      if (!onHome) {
        const tree = await dumpAccessibilityTree();
        console.log(`${LOG} Home not reached. Tree:\n`, tree.slice(0, 4000));
      }
      expect(onHome).toBe(true);
    }
  });

  // -------------------------------------------------------------------------
  // Phase 3: Logout returns to Welcome
  // -------------------------------------------------------------------------

  it('logout from Settings returns the user to Welcome', async () => {
    await logoutViaSettings(LOG);

    // logoutViaSettings already asserts the logged-out marker; double-check
    // the Welcome OAuth row reappeared so we know the route reset cleanly.
    expect(await waitForText('Welcome to OpenHuman', 15_000)).toBeTruthy();
    expect(await textExists('Select a Runtime')).toBe(true);
  });
});
