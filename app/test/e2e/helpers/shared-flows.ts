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
  const page = await browser.$('[data-testid="accounts-page"]');
  await page.waitForDisplayed({ timeout: 15_000 });

  const opened = await browser.execute(() => {
    const addBtn = document.querySelector<HTMLButtonElement>('[data-testid="accounts-add-button"]');
    if (!addBtn) return false;
    addBtn.click();
    return true;
  });
  if (!opened) {
    throw new Error('Could not locate Add Account button on /chat accounts page');
  }
  const modal = await browser.$('[data-testid="add-account-modal"]');
  await modal.waitForDisplayed({ timeout: 5_000 });
}

export async function waitForAccountsPage(timeout = 15_000): Promise<void> {
  const page = await browser.$('[data-testid="accounts-page"]');
  await page.waitForDisplayed({ timeout });
}

export async function clickAddAccountProvider(provider: string, timeout = 10_000): Promise<void> {
  const tile = await browser.$(`[data-testid="add-account-provider-${provider}"]`);
  await tile.waitForDisplayed({ timeout });
  await tile.click();
}

export async function waitForAddAccountModalClosed(timeout = 5_000): Promise<void> {
  const modal = await browser.$('[data-testid="add-account-modal"]');
  await modal.waitForExist({ timeout, reverse: true });
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
  // Home page (Home.tsx) renders t('home.askAssistant') = 'Ask your assistant anything...'
  // as a stable CTA button. The animated typewriter heading ('Welcome, <name> 👋' etc.)
  // and old strings ('Good morning', 'Message OpenHuman', 'Upgrade to Premium') are gone.
  const candidates = ['Ask your assistant anything', 'Your device is connected'];
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
  '/chat': 'Chat',
  '/notifications': 'Alerts',
  '/settings': 'Settings',
  '/intelligence': 'Intelligence',
};

function normalizeHash(value) {
  const raw = String(value || '');
  const withPrefix = raw.startsWith('#') ? raw : `#${raw}`;
  return withPrefix.replace(/\/$/, '');
}

function routeReadySelector(hash) {
  const path = normalizeHash(hash).replace(/^#/, '');
  const selectors = {
    '/notifications': '[data-testid="integration-notifications-section"]',
    '/settings/cron-jobs': '[data-testid="cron-jobs-panel"]',
    '/settings/privacy': '[data-testid="settings-privacy-panel"]',
    '/settings/migration': '[data-testid="migration-form"]',
    '/settings/voice': '[data-testid="voice-providers-section"]',
    '/settings/memory-data': '[data-testid="memory-workspace"]',
    '/intelligence': '[data-testid="memory-workspace"]',
  };
  return selectors[path] || null;
}

async function routeSignature() {
  return browser.execute(() => {
    const root = document.getElementById('root');
    return (root?.innerText || root?.textContent || '').trim().slice(0, 500);
  });
}

async function waitForHashRouteReady(hash, options = {}) {
  const { timeout = 10_000, previousSignature = '', allowSameSignature = false } = options;
  const expected = normalizeHash(hash);
  const readySelector = routeReadySelector(hash);
  await browser.waitUntil(
    async () =>
      Boolean(
        await browser.execute(
          ({ target, selector, before, allowSame }) => {
            if (document.readyState !== 'complete') return false;
            const current = window.location.hash.replace(/\/$/, '');
            if (current !== target) return false;
            const root = document.getElementById('root');
            if (!root) return false;
            if (selector && root.querySelector(selector)) return true;

            const signature = (root.innerText || root.textContent || '').trim().slice(0, 500);
            if (!signature) return false;
            return allowSame || signature !== before;
          },
          {
            target: expected,
            selector: readySelector,
            before: previousSignature,
            allowSame: allowSameSignature,
          }
        )
      ),
    {
      timeout,
      interval: 250,
      timeoutMsg: `hash route ${expected} did not become ready within ${timeout}ms`,
    }
  );
}

export async function navigateViaHash(hash) {
  const normalized = String(hash).replace(/\/$/, '') || hash;
  const expectedHash = `#${normalized}`;
  const hashMatches = currentHash =>
    currentHash === expectedHash || String(currentHash).startsWith(`${expectedHash}/`);
  const waitForHash = async (timeout = 8_000) =>
    browser.waitUntil(
      async () => {
        const currentHash = await browser.execute(() => window.location.hash);
        if (!hashMatches(currentHash)) return false;
        await browser.pause(300);
        const stableHash = await browser.execute(() => window.location.hash);
        return hashMatches(stableHash);
      },
      { timeout, interval: 250, timeoutMsg: `hash did not settle on ${hash}` }
    );

  if (supportsExecuteScript()) {
    // Try sidebar button click first — more reliable than direct hash set.
    const label = HASH_TO_SIDEBAR_LABEL[normalized];
    if (label) {
      try {
        const clicked = await browser.execute((targetLabel: string) => {
          const buttons = Array.from(document.querySelectorAll('button')) as HTMLButtonElement[];
          const button = buttons.find(btn => {
            const aria = btn.getAttribute('aria-label')?.trim();
            const title = btn.getAttribute('title')?.trim();
            const text = btn.textContent?.trim();
            return aria === targetLabel || title === targetLabel || text === targetLabel;
          });
          if (!button) return false;
          button.click();
          return true;
        }, label);
        if (clicked) {
          await waitForHash();
          const currentHash = await browser.execute(() => window.location.hash);
          console.log(`[E2E] Navigated to ${hash} via "${label}" (current: ${currentHash})`);
          return;
        }
      } catch (buttonErr) {
        console.log(`[E2E] Button navigation to ${hash} failed:`, buttonErr);
      }
    }

    // Fallback: direct hash set + wait for route readiness.
    try {
      const beforeSignature = await routeSignature();
      const beforeHash = normalizeHash(await browser.execute(() => window.location.hash));
      const targetHash = normalizeHash(hash);
      await browser.execute(h => {
        window.location.hash = h;
      }, hash);
      await waitForHashRouteReady(hash, {
        previousSignature: beforeSignature,
        allowSameSignature: beforeHash === targetHash,
      });
      const currentHash = await browser.execute(() => window.location.hash);
      console.log(`[E2E] Navigated to ${hash} (current: ${currentHash})`);
      return;
    } catch (err) {
      console.log(`[E2E] Hash navigation to ${hash} failed:`, err);
    }

    // Last resort: retry button click.
    if (label) {
      try {
        const clicked = await browser.execute((targetLabel: string) => {
          const buttons = Array.from(document.querySelectorAll('button')) as HTMLButtonElement[];
          const button = buttons.find(btn => {
            const aria = btn.getAttribute('aria-label')?.trim();
            const title = btn.getAttribute('title')?.trim();
            const text = btn.textContent?.trim();
            return aria === targetLabel || title === targetLabel || text === targetLabel;
          });
          if (!button) return false;
          button.click();
          return true;
        }, label);
        if (!clicked) {
          throw new Error(`could not find nav button "${label}"`);
        }
        await waitForHash();
        const currentHash = await browser.execute(() => window.location.hash);
        console.log(`[E2E] Navigated to ${hash} via "${label}" (current: ${currentHash})`);
        return;
      } catch (fallbackErr) {
        console.log(`[E2E] Button navigation to ${hash} failed:`, fallbackErr);
      }
    }

    throw new Error(`[E2E] Failed to navigate to ${hash}`);
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

  const billingMarkers = ['Billing moved to the web', 'Open billing dashboard', 'Open dashboard'];
  const deadline = Date.now() + 15_000;
  let hasBilling = false;
  while (Date.now() < deadline) {
    for (const marker of billingMarkers) {
      hasBilling = await textExists(marker);
      if (hasBilling) break;
    }
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
  let finalCheck = false;
  const finalDeadline = Date.now() + 15_000;
  while (Date.now() < finalDeadline) {
    for (const marker of billingMarkers) {
      finalCheck = await textExists(marker);
      if (finalCheck) break;
    }
    if (finalCheck) break;
    await browser.pause(500);
  }
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
  await navigateViaHash('/intelligence');
}

export async function navigateToConversations() {
  await navigateViaHash('/chat');
}

export async function navigateToNotifications() {
  await navigateViaHash('/notifications');
}

// ---------------------------------------------------------------------------
// Onboarding walkthrough
// Current flow: Welcome → Skills → optional Context gathering.
// ---------------------------------------------------------------------------

/** Labels used to detect the onboarding overlay (same strings as Onboarding copy). */
export const ONBOARDING_OVERLAY_TEXTS = [
  'Skip',
  'Welcome',
  "Hi. I'm OpenHuman.",
  "Let's Start",
  'Connect your Gmail',
  'Skip for Now',
  'Building your profile',
  'Almost there',
  'Continue to chat',
  'Run AI Models Locally',
  'Screen & Accessibility',
  'Enable Tools',
  'Install Skills',
] as const;

/** True when the routed full-screen onboarding flow is visible. */
async function onboardingOverlayLikelyVisible(): Promise<boolean> {
  if (supportsExecuteScript()) {
    const routedOnboarding = await browser.execute(() => {
      const onOnboardingRoute = window.location.hash.startsWith('#/onboarding');
      const hasOnboardingShell =
        document.querySelector('[data-testid="onboarding-layout"]') !== null ||
        document.querySelector('[data-testid="onboarding-next-button"]') !== null;
      return onOnboardingRoute && hasOnboardingShell;
    });
    if (routedOnboarding) return true;
  }

  for (const label of ONBOARDING_OVERLAY_TEXTS) {
    if (label === 'Welcome') continue;
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

export async function dismissWalkthroughIfVisible(timeout = 6_000): Promise<boolean> {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (supportsExecuteScript()) {
      const status = await browser.execute(() => {
        const buttons = Array.from(document.querySelectorAll<HTMLButtonElement>('button'));
        const skip = buttons.find(button => (button.textContent ?? '').trim() === 'Skip tour');
        if (!skip) return 'not-visible';
        ['mousedown', 'mouseup', 'click'].forEach(type => {
          skip.dispatchEvent(
            new MouseEvent(type, { bubbles: true, cancelable: true, view: window, button: 0 })
          );
        });
        return 'clicked';
      });
      if (status === 'clicked') {
        await browser.waitUntil(async () => !(await textExists('Skip tour')), {
          timeout: 4_000,
          interval: 250,
          timeoutMsg: 'walkthrough skip button remained visible',
        });
        return true;
      }
    } else if (await textExists('Skip tour')) {
      await clickText('Skip tour', 2_000);
      return true;
    }
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
      // The BootCheckGate renders a full-screen `.fixed` overlay with a
      // heading. Check for both "Choose core mode" (legacy) and
      // "Select a Runtime" (current i18n key bootCheck.chooseCoreMode).
      // Important: only match headings inside a `.fixed` overlay — the
      // Welcome page also has a "Select a Runtime" button, but that is
      // NOT the BootCheckGate and clicking it would reset the core mode.
      const heading = Array.from(document.querySelectorAll('.fixed h2')).find(h => {
        const text = (h.textContent ?? '').trim();
        return text === 'Choose core mode' || text === 'Select a Runtime';
      });
      if (!heading) return 'gone';
      const modal = heading.closest('.fixed') ?? heading.parentElement;
      if (!modal) return 'gone';
      const buttons = Array.from(modal.querySelectorAll<HTMLButtonElement>('button'));
      const primary =
        buttons.find(b => (b.textContent ?? '').trim() === 'Continue') ??
        buttons.find(b => (b.textContent ?? '').trim().includes('Local')) ??
        buttons.find(b => /bg-ocean-500|bg-primary/.test(b.className)) ??
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

async function waitForPostOnboardingHome(logPrefix, timeout = 20_000) {
  if (supportsExecuteScript()) {
    // After onboarding the app routes to either #/home or #/chat depending on
    // the DefaultRedirect guard and the user's onboarding state. Accept both.
    await browser.waitUntil(
      async () =>
        Boolean(
          await browser.execute(() => {
            const h = window.location.hash.replace(/\/$/, '');
            return h === '#/home' || h === '#/chat';
          })
        ),
      {
        timeout: Math.min(timeout, 10_000),
        interval: 300,
        timeoutMsg: 'onboarding completed but hash did not settle on #/home or #/chat',
      }
    );
  }

  // Check for Home page markers, but don't fail if we're on /chat instead.
  const homeText = await waitForHomePage(Math.min(timeout, 8_000));
  if (!homeText) {
    // The app may have routed to /chat. Check for chat markers.
    const onChat =
      supportsExecuteScript() &&
      (await browser.execute(() => window.location.hash.startsWith('#/chat')));
    if (onChat) {
      console.log(`${logPrefix} Post-onboarding landed on /chat (accepted)`);
      return;
    }
    const tree = await dumpAccessibilityTree();
    console.log(`${logPrefix} Home page not ready after onboarding. Tree:\n`, tree.slice(0, 4000));
    throw new Error('Onboarding dismissed but Home page did not become ready');
  }

  console.log(`${logPrefix} Post-onboarding Home page confirmed: found "${homeText}"`);
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
    await dismissWalkthroughIfVisible(3_000);
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
      await waitForPostOnboardingHome(logPrefix);
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
  await dismissWalkthroughIfVisible(8_000);
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
  await waitForHomePage(15_000);
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
  // Logout + Clear App Data moved out of the main /settings page and into
  // the Account section in PR #2550 (LogoutAndClearActions footer on
  // /settings/account). Navigate straight to the section that actually
  // renders the buttons.
  await navigateViaHash('/settings/account');

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
