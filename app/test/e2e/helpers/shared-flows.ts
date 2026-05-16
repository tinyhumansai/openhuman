// @ts-nocheck
/**
 * Shared E2E flow helpers for Linux (tauri-driver).
 *
 * Extracted from individual spec files to avoid duplication.
 * All navigation uses browser.execute() with window.location.hash
 * because sidebar nav buttons are icon-only (aria-label, no text content).
 */
import { waitForAppReady, waitForAuthBootstrap } from './app-helpers';
import { triggerAuthDeepLink } from './deep-link-helpers';
import {
  clickText,
  dumpAccessibilityTree,
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from './element-helpers';
import { supportsExecuteScript } from './platform';

// ---------------------------------------------------------------------------
// Accounts page helpers
// ---------------------------------------------------------------------------

/**
 * Open the "Add Account" modal on /accounts.
 *
 * The "Add app" affordance is a button whose only labelled descendants are an
 * SVG plus a tooltip span with `pointer-events: none`. None of the shared
 * `clickButton`/`clickText` helpers can target it cleanly because the
 * accessible name lives only on `aria-label`, so this helper reaches for the
 * explicit selector. Tracking a follow-up `clickByAriaLabel` helper.
 */
export async function openAddAccountModal(): Promise<void> {
  const opened = await browser.execute(() => {
    const buttons = Array.from(document.querySelectorAll<HTMLButtonElement>('button'));
    const addBtn = buttons.find(b => b.getAttribute('aria-label') === 'Add app');
    if (addBtn) {
      addBtn.click();
      return true;
    }
    return false;
  });
  if (!opened) {
    throw new Error('Could not locate Add app button on /accounts');
  }
  await waitForText('Add account', 5_000);
}

// ---------------------------------------------------------------------------
// Generic helpers
// ---------------------------------------------------------------------------

export async function waitForRequest(log, method, urlFragment, timeout = 15_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const match = log().find(r => r.method === method && r.url.includes(urlFragment));
    if (match) return match;
    await browser.pause(500);
  }
  return undefined;
}

export async function waitForHomePage(timeout = 15_000) {
  const candidates = [
    'Test',
    'Good morning',
    'Good afternoon',
    'Good evening',
    'Message OpenHuman',
    'Upgrade to Premium',
  ];
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    for (const text of candidates) {
      if (await textExists(text)) return text;
    }
    await browser.pause(1_000);
  }
  return null;
}

export async function waitForTextToDisappear(text, timeout = 10_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (!(await textExists(text))) return true;
    await browser.pause(500);
  }
  return false;
}

/**
 * Click the first matching text from a list of candidates.
 */
export async function clickFirstMatch(candidates, timeout = 5_000) {
  for (const text of candidates) {
    if (await textExists(text)) {
      await clickText(text, timeout);
      return text;
    }
  }
  return null;
}

// ---------------------------------------------------------------------------
// Navigation helpers (JS hash-based — icon-only sidebar buttons)
// ---------------------------------------------------------------------------

/** Appium Mac2 cannot run W3C Execute Script in WKWebView — use sidebar labels instead. */
const HASH_TO_SIDEBAR_LABEL = {
  '/skills': 'Skills',
  '/home': 'Home',
  '/conversations': 'Conversations',
  '/notifications': 'Alerts',
  '/settings': 'Settings',
  '/settings/intelligence': 'Intelligence',
};

export async function navigateViaHash(hash) {
  const normalized = String(hash).replace(/\/$/, '') || hash;

  if (supportsExecuteScript()) {
    try {
      await browser.execute(h => {
        window.location.hash = h;
      }, hash);
      await browser.pause(2_000);
      const currentHash = await browser.execute(() => window.location.hash);
      console.log(`[E2E] Navigated to ${hash} (current: ${currentHash})`);
    } catch (err) {
      console.log(`[E2E] Hash navigation to ${hash} failed:`, err);
    }
    return;
  }

  // Appium Mac2 — Settings → Billing (nested route)
  if (normalized === '/settings/billing') {
    try {
      await clickText('Settings', 12_000);
      await browser.pause(1_500);
      const sub = await clickFirstMatch(['Billing & Usage', 'Billing'], 12_000);
      if (!sub) {
        throw new Error('Mac2: could not find Billing / Billing & Usage after opening Settings');
      }
      await browser.pause(2_000);
      console.log(`[E2E] Mac2 navigated to ${hash} via Settings → ${sub}`);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      throw new Error(`[E2E] Mac2: failed to navigate to ${hash}: ${msg}`);
    }
    return;
  }

  const label = HASH_TO_SIDEBAR_LABEL[normalized];
  if (label) {
    try {
      await clickText(label, 12_000);
      await browser.pause(2_000);
      console.log(`[E2E] Mac2 sidebar navigation to ${hash} via "${label}"`);
    } catch (err) {
      console.log(`[E2E] Mac2 sidebar navigation to ${hash} failed:`, err);
    }
    return;
  }

  throw new Error(
    `[E2E] Mac2: no sidebar mapping for hash "${hash}". Extend HASH_TO_SIDEBAR_LABEL or add a branch in navigateViaHash.`
  );
}

export async function navigateToHome() {
  await navigateViaHash('/home');
  const homeText = await waitForHomePage(10_000);
  if (!homeText) {
    if (supportsExecuteScript()) {
      try {
        await browser.execute(() => {
          window.location.hash = '/home';
        });
      } catch {
        /* ignore */
      }
    } else {
      try {
        await clickText('Home', 8_000);
      } catch {
        /* ignore */
      }
    }
    await browser.pause(2_000);
    await waitForHomePage(10_000);
  }
}

export async function navigateToSettings() {
  await navigateViaHash('/settings');
}

export async function navigateToBilling() {
  await navigateViaHash('/settings/billing');

  const deadline = Date.now() + 15_000;
  let hasBilling = false;
  while (Date.now() < deadline) {
    hasBilling =
      (await textExists('Current Plan')) ||
      (await textExists('FREE')) ||
      (await textExists('Upgrade'));
    if (hasBilling) break;
    await browser.pause(500);
  }

  if (hasBilling) {
    console.log('[E2E] Billing page loaded');
    return;
  }

  console.log('[E2E] Billing content not found after initial navigation; running fallback');

  await navigateViaHash('/settings');
  await browser.pause(3_000);

  if (supportsExecuteScript()) {
    const currentHash = await browser.execute(() => window.location.hash);
    console.log(`[E2E] Billing fallback: current hash ${currentHash}`);

    const clicked = await browser.execute(() => {
      const allText = document.querySelectorAll('*');
      for (const el of allText) {
        const text = el.textContent?.trim() || '';
        if (
          (text === 'Billing & Usage' || text === 'Billing') &&
          el.closest('button, [role="button"], a, [class*="MenuItem"]')
        ) {
          (el.closest('button, [role="button"], a, [class*="MenuItem"]') as HTMLElement).click();
          return 'clicked';
        }
      }
      window.location.hash = '/settings/billing';
      return 'hash-fallback';
    });
    console.log(`[E2E] Billing fallback: ${clicked}`);
  } else {
    const sub = await clickFirstMatch(['Billing & Usage', 'Billing'], 10_000);
    console.log(`[E2E] Billing fallback (Mac2): clicked ${sub}`);
  }
  await browser.pause(3_000);

  // Verify billing actually loaded after fallback
  const finalCheck =
    (await textExists('Current Plan')) ||
    (await textExists('FREE')) ||
    (await textExists('Upgrade'));
  if (!finalCheck) {
    let finalHash = '';
    if (supportsExecuteScript()) {
      finalHash = await browser.execute(() => window.location.hash);
    }
    const tree = await dumpAccessibilityTree();
    console.log(`[E2E] Billing verification failed after fallback. Hash: ${finalHash}`);
    console.log(`[E2E] Accessibility tree:\n`, tree.slice(0, 4000));
    throw new Error(
      `navigateToBilling: billing markers not found after fallback (hash: ${finalHash})`
    );
  }
  console.log('[E2E] Billing page loaded (after fallback)');
}

export async function navigateToSkills() {
  await navigateViaHash('/skills');
}

export async function navigateToIntelligence() {
  await navigateViaHash('/settings/intelligence');
}

export async function navigateToConversations() {
  await navigateViaHash('/conversations');
}

export async function navigateToNotifications() {
  await navigateViaHash('/notifications');
}

// ---------------------------------------------------------------------------
// Onboarding walkthrough
// Current flow: Welcome → Local AI → Screen & Accessibility → Tools → Skills (5 steps, indices 0–4).
// ---------------------------------------------------------------------------

/** Labels used to detect the onboarding overlay (same strings as Onboarding copy). */
export const ONBOARDING_OVERLAY_TEXTS = [
  'Skip',
  'Welcome',
  'Run AI Models Locally',
  'Screen & Accessibility',
  'Enable Tools',
  'Install Skills',
] as const;

/** True when the full-screen onboarding overlay is likely visible. */
async function onboardingOverlayLikelyVisible(): Promise<boolean> {
  for (const label of ONBOARDING_OVERLAY_TEXTS) {
    if (await textExists(label)) return true;
  }
  return false;
}

export async function isOnboardingOverlayVisible(): Promise<boolean> {
  return onboardingOverlayLikelyVisible();
}

export async function waitForOnboardingOverlayVisible(timeout = 10_000): Promise<boolean> {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await onboardingOverlayLikelyVisible()) return true;
    await browser.pause(400);
  }
  return false;
}

export async function waitForOnboardingOverlayHidden(timeout = 10_000): Promise<boolean> {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (!(await onboardingOverlayLikelyVisible())) return true;
    await browser.pause(400);
  }
  return false;
}

/**
 * BootCheckGate shows a "Choose core mode" modal on fresh storage. It sits
 * *in front of* the routed page, so onboarding never mounts behind it. We
 * click the primary "Continue" CTA via a synthetic MouseEvent and retry
 * until the modal is gone (a single click can race against the gate's
 * re-render). Exported so specs that bypass `walkOnboarding` can still
 * call this directly.
 */
export async function dismissBootCheckGateIfVisible(timeoutMs = 12_000): Promise<boolean> {
  if (!supportsExecuteScript()) return false;
  const deadline = Date.now() + timeoutMs;
  let everSeen = false;
  while (Date.now() < deadline) {
    const status = await browser.execute(() => {
      const heading = Array.from(document.querySelectorAll('h2')).find(
        h => (h.textContent ?? '').trim() === 'Choose core mode'
      );
      if (!heading) return 'gone';
      const modal = heading.closest('.fixed') ?? heading.parentElement;
      if (!modal) return 'gone';
      const buttons = Array.from(modal.querySelectorAll<HTMLButtonElement>('button'));
      const primary =
        buttons.find(b => (b.textContent ?? '').trim() === 'Continue') ??
        buttons.find(b => /bg-ocean-500/.test(b.className)) ??
        buttons[buttons.length - 1];
      if (!primary) return 'visible-no-button';
      ['mousedown', 'mouseup', 'click'].forEach(type => {
        primary.dispatchEvent(
          new MouseEvent(type, { bubbles: true, cancelable: true, view: window, button: 0 })
        );
      });
      return 'clicked';
    });
    if (status === 'gone') return everSeen;
    everSeen = true;
    await browser.pause(800);
  }
  return everSeen;
}

/**
 * Walk through onboarding by advancing the `data-testid="onboarding-next-button"`
 * until it unmounts. The button is rendered on every step (see
 * app/src/pages/onboarding/components/OnboardingNextButton.tsx), so we don't
 * need to track step *titles* — title-based detection silently skipped any
 * step that wasn't in `ONBOARDING_OVERLAY_TEXTS` (e.g. "Connect your Gmail")
 * and left specs wedged behind onboarding while reporting success.
 *
 * We dispatch a real synthetic MouseEvent so React's onClick fires reliably,
 * and bail out if the button gets stuck in a permanently-disabled state.
 *
 * Dismisses BootCheckGate ("Choose core mode") first if it's blocking the
 * route — onboarding sits behind it, so without this the walker just times
 * out waiting for the next-button to mount.
 */
export async function walkOnboarding(logPrefix = '[E2E]', maxSteps = 12): Promise<void> {
  if (!supportsExecuteScript()) {
    // Mac2/no-script fallback: legacy "Continue" matcher, kept so the
    // unsupported-driver branch isn't a hard failure for old harnesses.
    const clicked = await clickFirstMatch(['Continue'], 3_000);
    if (clicked) console.log(`${logPrefix} Onboarding: clicked Continue (legacy fallback)`);
    return;
  }

  // Onboarding mounts beneath BootCheckGate. If the user is fresh-installed
  // the gate is up and onboarding will never render until we confirm it.
  const dismissed = await dismissBootCheckGateIfVisible();
  if (dismissed) {
    console.log(`${logPrefix} Dismissed BootCheckGate before onboarding`);
    await browser.pause(1_500);
  }

  // Wait up to 15s for the onboarding shell to actually mount. If the user is
  // already onboarded (e.g. resuming an existing session) the button never
  // appears and we return without firing any clicks.
  const appeared = await browser
    .waitUntil(
      async () =>
        Boolean(
          await browser.execute(
            () => document.querySelector('[data-testid="onboarding-next-button"]') !== null
          )
        ),
      { timeout: 15_000, interval: 500, timeoutMsg: 'onboarding-next-button never appeared' }
    )
    .catch(() => false);

  if (!appeared) {
    console.log(`${logPrefix} Onboarding next-button never appeared — assuming already onboarded`);
    return;
  }

  for (let step = 0; step < maxSteps; step += 1) {
    const status = await browser.execute(() => {
      const btn = document.querySelector<HTMLButtonElement>(
        '[data-testid="onboarding-next-button"]'
      );
      const onOnboardingHash = window.location.hash.startsWith('#/onboarding');
      if (!btn) return onOnboardingHash ? 'gone-but-onboarding-hash' : 'gone';
      if (btn.disabled) return 'disabled';
      ['mousedown', 'mouseup', 'click'].forEach(type => {
        btn.dispatchEvent(
          new MouseEvent(type, { bubbles: true, cancelable: true, view: window, button: 0 })
        );
      });
      return 'clicked';
    });

    if (status === 'gone') {
      console.log(`${logPrefix} Onboarding dismissed after ${step} step(s)`);
      return;
    }
    if (status === 'gone-but-onboarding-hash') {
      // The button momentarily unmounts between steps (animation / lazy render).
      // Don't claim victory yet — wait for the next step to render.
      console.log(
        `${logPrefix} Onboarding next-button absent but hash still on /onboarding — waiting`
      );
      await browser.pause(1_500);
      continue;
    }
    if (status === 'disabled') {
      // Some steps gate the button on async work (skill catalog fetch, local
      // AI download check). Give it a beat, then retry; if it stays disabled
      // for too long we bail rather than spinning forever.
      console.log(`${logPrefix} Onboarding step ${step}: next-button disabled — waiting`);
      await browser.pause(2_000);
      continue;
    }
    console.log(`${logPrefix} Onboarding step ${step}: clicked Continue`);
    await browser.pause(step >= 4 ? 3_000 : 1_500);
  }
  console.log(`${logPrefix} Onboarding hit max steps (${maxSteps}) — moving on`);
}

/**
 * Walk through onboarding if it is visible, or no-op if already on Home.
 *
 * Delegates to walkOnboarding, which polls up to 8 × 400 ms for the overlay
 * to appear before giving up — safe to call unconditionally after auth so
 * timing races do not cause the helper to skip onboarding prematurely.
 */
export async function completeOnboardingIfVisible(logPrefix = '[E2E]') {
  await walkOnboarding(logPrefix);
}

export async function waitForLoggedOutState(timeout = 10_000): Promise<string | null> {
  const welcomeCandidates = ['Welcome', 'Sign in', 'Login', 'Get Started'];
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    for (const text of welcomeCandidates) {
      if (await textExists(text)) {
        return text;
      }
    }
    await browser.pause(500);
  }
  return null;
}

export async function logoutViaSettings(logPrefix = '[E2E]') {
  await navigateToSettings();

  const loggedOut = await browser.execute(() => {
    const candidates = ['Log out', 'Logout', 'Sign out'];
    const allElements = document.querySelectorAll('*');
    for (const label of candidates) {
      for (const el of allElements) {
        const text = el.textContent?.trim() || '';
        if (text !== label) continue;
        const clickable = el.closest(
          'button, [role="button"], a, [class*="MenuItem"]'
        ) as HTMLElement | null;
        if (clickable) {
          clickable.click();
          return label;
        }
        (el as HTMLElement).click();
        return label;
      }
    }
    return null;
  });

  if (!loggedOut) {
    const clicked = await clickFirstMatch(['Log out', 'Logout', 'Sign out'], 10_000);
    if (!clicked) {
      const tree = await dumpAccessibilityTree();
      console.log(`${logPrefix} Logout button not found. Tree:\n`, tree.slice(0, 4000));
      throw new Error('Could not find logout button in Settings');
    }
    console.log(`${logPrefix} Logout clicked via text helper: "${clicked}"`);
  } else {
    console.log(`${logPrefix} Logout clicked: "${loggedOut}"`);
  }

  await browser.pause(2_000);

  const hasConfirm =
    (await textExists('Confirm')) || (await textExists('Yes')) || (await textExists('Log Out'));
  if (hasConfirm) {
    const confirmed = await browser.execute(() => {
      const candidates = document.querySelectorAll('button, [role="button"], a');
      for (const el of candidates) {
        const text = el.textContent?.trim() || '';
        const label = el.getAttribute('aria-label') || '';
        if (
          ['Confirm', 'Yes', 'Log Out'].some(candidate => text === candidate || label === candidate)
        ) {
          (el as HTMLElement).click();
          return true;
        }
      }
      return false;
    });
    if (!confirmed) {
      throw new Error('Logout confirmation dialog appeared but confirm button was not clickable');
    }
    console.log(`${logPrefix} Logout confirmation accepted`);
  }

  const loggedOutMarker = await waitForLoggedOutState(10_000);
  if (!loggedOutMarker) {
    const tree = await dumpAccessibilityTree();
    console.log(`${logPrefix} Logged-out state not detected. Tree:\n`, tree.slice(0, 4000));
    throw new Error('Logged-out state was not visible after logout');
  }

  console.log(`${logPrefix} Logged-out state confirmed: "${loggedOutMarker}"`);
}

// ---------------------------------------------------------------------------
// Full login flow
// ---------------------------------------------------------------------------

/**
 * @param token          Deep link token string.
 * @param logPrefix      Prefix for console log lines.
 * @param postLoginVerifier  Optional async callback invoked after the Home page
 *   is confirmed.  Receives `logPrefix` so it can log consistently.  If the
 *   verifier throws, performFullLogin propagates the error — callers can use
 *   this to assert that auth side-effects (e.g. token consume, profile fetch)
 *   actually occurred rather than relying on UI alone.
 */
export async function performFullLogin(
  token = 'e2e-test-token',
  logPrefix = '[E2E]',
  postLoginVerifier?: (logPrefix: string) => Promise<void>
) {
  await triggerAuthDeepLink(token);
  await waitForWindowVisible(25_000);
  await waitForWebView(15_000);
  await waitForAppReady(15_000);
  await waitForAuthBootstrap(15_000);

  await walkOnboarding(logPrefix);

  const homeText = await waitForHomePage(15_000);
  if (!homeText) {
    const tree = await dumpAccessibilityTree();
    console.log(`${logPrefix} Home page not reached after login. Tree:\n`, tree.slice(0, 4000));
    throw new Error('Full login did not reach Home page');
  }

  if (postLoginVerifier) {
    await postLoginVerifier(logPrefix);
  }

  console.log(`${logPrefix} Home page confirmed: found "${homeText}"`);
}
