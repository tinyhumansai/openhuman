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
  '/settings': 'Settings',
  '/intelligence': 'Intelligence',
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
  'Welcome',
  'Run AI Models Locally',
  'Screen & Accessibility',
  'Enable Tools',
  'Install Skills',
  'Lastly, your Recovery Phrase',
  'Your Recovery Phrase',
  'Import Recovery Phrase',
  'Finish Setup',
] as const;

/** True when the full-screen onboarding overlay is likely visible. */
async function onboardingOverlayLikelyVisible(): Promise<boolean> {
  for (const label of ONBOARDING_OVERLAY_TEXTS) {
    if (await textExists(label)) return true;
  }
  return false;
}

async function completeMnemonicStep(logPrefix: string): Promise<boolean> {
  const mnemonicVisible =
    (await textExists('Lastly, your Recovery Phrase')) ||
    (await textExists('Your Recovery Phrase')) ||
    (await textExists('Import Recovery Phrase'));

  if (!mnemonicVisible) {
    return false;
  }

  console.log(`${logPrefix} MnemonicStep visible`);

  try {
    const checked = await browser.execute(() => {
      const checkbox = document.querySelector('input[type="checkbox"]') as HTMLInputElement | null;
      if (!checkbox) return false;
      if (!checkbox.checked) {
        checkbox.click();
      }
      return checkbox.checked;
    });
    console.log(`${logPrefix} MnemonicStep checkbox checked=${checked}`);
  } catch (err) {
    console.log(`${logPrefix} MnemonicStep checkbox interaction failed:`, err);
  }

  const clicked = await clickFirstMatch(['Finish Setup', 'Continue'], 12_000);
  if (clicked) {
    console.log(`${logPrefix} MnemonicStep: clicked ${clicked}`);
    await browser.pause(4_000);
    return true;
  }

  return false;
}

/**
 * Walk through onboarding. Supports both the legacy 5-step flow and the
 * newer 6-step variant that ends with a recovery phrase confirmation.
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

  // Up to 7 passes covers the 6-step flow plus one retry while async content settles.
  for (let step = 0; step < 7; step++) {
    if (!(await onboardingOverlayLikelyVisible())) {
      console.log(`${logPrefix} Onboarding dismissed after step ${step}`);
      return;
    }

    if (await completeMnemonicStep(logPrefix)) {
      continue;
    }

    const clicked = await clickFirstMatch(['Continue', 'Finish Setup'], 12_000);
    if (clicked) {
      console.log(`${logPrefix} Onboarding step ${step}: clicked ${clicked}`);
      await browser.pause(clicked === 'Finish Setup' || step >= 4 ? 4_000 : 2_000);
    } else {
      const installSkillsLabel = ONBOARDING_OVERLAY_TEXTS[ONBOARDING_OVERLAY_TEXTS.length - 1]!;
      if (
        (await textExists('Install Skills')) ||
        (await textExists('Finish Setup')) ||
        (await textExists(installSkillsLabel))
      ) {
        await browser.pause(2_500);
        if (await completeMnemonicStep(logPrefix)) {
          continue;
        }
        const retry = await clickFirstMatch(['Continue', 'Finish Setup'], 10_000);
        if (retry) {
          console.log(`${logPrefix} Onboarding step ${step}: retry ${retry}`);
          await browser.pause(4_000);
          continue;
        }
      }
      const tree = await dumpAccessibilityTree();
      console.log(`${logPrefix} Onboarding stalled at step ${step}. Tree:\n`, tree.slice(0, 4000));
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
