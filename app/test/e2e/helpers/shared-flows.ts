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

export async function navigateViaHash(hash) {
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
}

export async function navigateToHome() {
  await navigateViaHash('/home');
  const homeText = await waitForHomePage(10_000);
  if (!homeText) {
    try {
      await browser.execute(() => {
        window.location.hash = '/home';
      });
    } catch {
      /* ignore */
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

  // Fallback
  const currentHash = await browser.execute(() => window.location.hash);
  console.log(`[E2E] Billing content not found. Current hash: ${currentHash}`);

  await navigateViaHash('/settings');
  await browser.pause(3_000);

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
  await browser.pause(3_000);

  // Verify billing actually loaded after fallback
  const finalCheck =
    (await textExists('Current Plan')) ||
    (await textExists('FREE')) ||
    (await textExists('Upgrade'));
  if (!finalCheck) {
    const finalHash = await browser.execute(() => window.location.hash);
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
  await navigateViaHash('/conversations');
}

// ---------------------------------------------------------------------------
// Onboarding walkthrough (Onboarding.tsx — 5 steps, indices 0–4)
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

/**
 * Walk through onboarding: Welcome → Local AI → Screen & Accessibility → Tools → Skills.
 * Each step uses the shared primary button label "Continue" (see OnboardingNextButton).
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

  // Up to 6 "Continue" clicks — covers 5 steps plus one retry if the list is still loading.
  for (let step = 0; step < 6; step++) {
    if (!(await onboardingOverlayLikelyVisible())) {
      console.log(`${logPrefix} Onboarding dismissed after step ${step}`);
      return;
    }

    const clicked = await clickFirstMatch(['Continue'], 12_000);
    if (clicked) {
      console.log(`${logPrefix} Onboarding step ${step}: clicked Continue`);
      await browser.pause(step >= 4 ? 4_000 : 2_000);
    } else {
      const installSkillsLabel = ONBOARDING_OVERLAY_TEXTS[ONBOARDING_OVERLAY_TEXTS.length - 1]!;
      if (await textExists(installSkillsLabel)) {
        await browser.pause(2_500);
        const retry = await clickFirstMatch(['Continue'], 10_000);
        if (retry) {
          console.log(
            `${logPrefix} Onboarding step ${step}: retry Continue on ${installSkillsLabel}`
          );
          await browser.pause(4_000);
        }
      }
      break;
    }
  }
}

/**
 * If onboarding is showing, walk through it. Safe no-op when already on Home / no overlay.
 */
export async function completeOnboardingIfVisible(logPrefix = '[E2E]') {
  if (await onboardingOverlayLikelyVisible()) {
    await walkOnboarding(logPrefix);
  }
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
