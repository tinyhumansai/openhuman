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
    'No messages yet',
    'Type a message',
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
  '/conversations': 'Chat',
  '/settings': 'Settings',
  '/intelligence': 'Intelligence',
  '/channels': 'Channels',
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

  // Appium Mac2 — nested settings routes via Skills built-in cards
  const SKILLS_BUILTIN_ROUTES: Record<string, string[]> = {
    '/settings/screen-intelligence': ['Screen Intelligence'],
    '/settings/voice': ['Voice Intelligence', 'Voice Dictation'],
    '/settings/autocomplete': ['Text Auto-Complete', 'Inline Autocomplete'],
  };
  const builtInLabels = SKILLS_BUILTIN_ROUTES[normalized];
  if (builtInLabels) {
    try {
      // Navigate to Skills page first, then click the built-in card
      await clickText('Skills', 12_000);
      await browser.pause(2_000);
      const sub = await clickFirstMatch(builtInLabels, 12_000);
      if (!sub) {
        // Fallback: try Settings sidebar → Automation menu item
        await clickText('Settings', 12_000);
        await browser.pause(1_500);
        const settingsSub = await clickFirstMatch(builtInLabels, 12_000);
        if (!settingsSub) {
          throw new Error(`Mac2: could not find ${builtInLabels.join(' / ')} in Skills or Settings`);
        }
        await browser.pause(2_000);
        console.log(`[E2E] Mac2 navigated to ${hash} via Settings → ${settingsSub}`);
        return;
      }
      await browser.pause(2_000);
      console.log(`[E2E] Mac2 navigated to ${hash} via Skills → ${sub}`);
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

/**
 * Dismiss the LocalAIDownloadSnackbar floating card if it is visible.
 *
 * The snackbar sits fixed bottom-right over the UI and can intercept clicks
 * on skill action buttons below it.  Call this before interacting with Skills.
 *
 * Two forms:
 *  - Expanded: has "Dismiss download notification" button (the ✕)
 *  - Collapsed pill: has "Expand download progress" button — less likely to overlap
 */
export async function dismissLocalAISnackbarIfVisible(logPrefix = '[E2E]') {
  try {
    // Try the X / dismiss button (visible when expanded)
    if (await textExists('Dismiss download notification')) {
      await clickText('Dismiss download notification', 5_000);
      await browser.pause(800);
      console.log(`${logPrefix} Dismissed LocalAI download snackbar`);
      return;
    }

    // Snackbar status texts that indicate it is expanded
    const snackbarTexts = [
      'Loading model...',
      'Downloading',
      'Installing Runtime',
      'Needs Attention',
      'Idle',
      'Ready',
    ];
    for (const text of snackbarTexts) {
      if (await textExists(text)) {
        // Dismiss button should now be accessible
        if (await textExists('Dismiss download notification')) {
          await clickText('Dismiss download notification', 5_000);
          await browser.pause(800);
          console.log(`${logPrefix} Dismissed LocalAI snackbar (state: ${text})`);
        } else if (await textExists('Collapse download progress')) {
          // Collapse to pill so it stops covering buttons
          await clickText('Collapse download progress', 5_000);
          await browser.pause(500);
          console.log(`${logPrefix} Collapsed LocalAI snackbar to pill (state: ${text})`);
        }
        return;
      }
    }
  } catch {
    // Non-fatal — snackbar may not be present
  }
}

export async function navigateToSkills() {
  await navigateViaHash('/skills');
}

export async function navigateToIntelligence() {
  await navigateViaHash('/intelligence');
}

export async function navigateToConversations() {
  await navigateViaHash('/conversations');
}

// ---------------------------------------------------------------------------
// Onboarding walkthrough
// Current flow: Welcome → Local AI → Screen & Accessibility → Tools → Skills (5 steps, indices 0–4).
// ---------------------------------------------------------------------------

/** Labels used to detect the onboarding overlay (same strings as Onboarding copy). */
export const ONBOARDING_OVERLAY_TEXTS = [
  'Skip',
  'Welcome On Board',
  'Let\'s Start',
  'referral code',
  'Skip for now',
  'Screen & Accessibility',
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
 * Walk through onboarding steps:
 *   Step 0: WelcomeStep        → "Let's Start"
 *   Step 1: ReferralApplyStep  → "Skip for now" (may be auto-skipped)
 *   Step 2: ScreenPermissions  → "Continue"
 *   Step 3: SkillsStep         → "Continue"
 *
 * Completing the last step dismisses the overlay.
 */
export async function walkOnboarding(logPrefix = '[E2E]') {
  let visible = false;
  for (let attempt = 0; attempt < 8; attempt++) {
    if (await onboardingOverlayLikelyVisible()) {
      visible = true;
      break;
    }
    await browser.pause(400);
  }

  if (!visible) {
    console.log(`${logPrefix} Onboarding overlay not visible — skipping`);
    await browser.pause(1_000);
    return;
  }

  // Step 0: WelcomeStep — click "Let's Start"
  {
    const clicked = await clickFirstMatch(["Let's Start"], 12_000);
    if (clicked) {
      console.log(`${logPrefix} Onboarding WelcomeStep: clicked "${clicked}"`);
      await browser.pause(2_000);
    }
  }

  if (!(await onboardingOverlayLikelyVisible())) {
    console.log(`${logPrefix} Onboarding dismissed after WelcomeStep`);
    return;
  }

  // Step 1: ReferralApplyStep — may be auto-skipped; click "Skip for now" if visible
  {
    const isReferral =
      (await textExists('referral code')) || (await textExists('Skip for now'));
    if (isReferral) {
      const clicked = await clickFirstMatch(['Skip for now', 'Continue'], 10_000);
      if (clicked) {
        console.log(`${logPrefix} Onboarding ReferralStep: clicked "${clicked}"`);
        await browser.pause(2_000);
      }
    }
  }

  // Steps 2-3: ScreenPermissions + SkillsStep — both use "Continue"
  for (let step = 2; step <= 3; step++) {
    if (!(await onboardingOverlayLikelyVisible())) {
      console.log(`${logPrefix} Onboarding dismissed after step ${step - 1}`);
      return;
    }

    const clicked = await clickFirstMatch(['Continue'], 12_000);
    if (clicked) {
      console.log(`${logPrefix} Onboarding step ${step}: clicked Continue`);
      await browser.pause(step === 3 ? 4_000 : 2_000);
    } else {
      // SkillsStep may take time to load — retry once
      if (await textExists('Install Skills')) {
        await browser.pause(2_500);
        const retry = await clickFirstMatch(['Continue'], 10_000);
        if (retry) {
          console.log(`${logPrefix} Onboarding step ${step}: retry Continue on Install Skills`);
          await browser.pause(4_000);
        }
      }
      break;
    }
  }
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
  let homeText: string | null = null;
  for (let attempt = 1; attempt <= 2; attempt += 1) {
    if (attempt > 1) {
      console.log(`${logPrefix} Retrying full login via deep link (attempt ${attempt}/2)`);
    }

    await triggerAuthDeepLink(token);
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await waitForAuthBootstrap(15_000);
    await walkOnboarding(logPrefix);

    homeText = await waitForHomePage(15_000);
    if (homeText) {
      break;
    }

    const loggedOutMarker = await waitForLoggedOutState(2_000);
    if (loggedOutMarker) {
      console.log(
        `${logPrefix} Login retry condition met — still on logged-out UI ("${loggedOutMarker}")`
      );
      continue;
    }
    break;
  }

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
