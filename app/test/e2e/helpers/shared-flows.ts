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
    await browser.execute((h) => { window.location.hash = h; }, hash);
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
      await browser.execute(() => { window.location.hash = '/home'; });
    } catch { /* ignore */ }
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
    hasBilling = (await textExists('Current Plan')) ||
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
      if ((text === 'Billing & Usage' || text === 'Billing') &&
          el.closest('button, [role="button"], a, [class*="MenuItem"]')) {
        (el.closest('button, [role="button"], a, [class*="MenuItem"]') as HTMLElement).click();
        return 'clicked';
      }
    }
    window.location.hash = '/settings/billing';
    return 'hash-fallback';
  });
  console.log(`[E2E] Billing fallback: ${clicked}`);
  await browser.pause(3_000);
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
// Onboarding walkthrough (Onboarding.tsx — 6 real steps)
// ---------------------------------------------------------------------------

/**
 * Walk through the real onboarding steps:
 *   Step 0: WelcomeStep       — "Continue"
 *   Step 1: LocalAIStep       — "Setup later" → "Continue" (skip Ollama)
 *   Step 2: ScreenPermissions — "Continue Without Permission"
 *   Step 3: ToolsStep         — "Continue"
 *   Step 4: SkillsStep        — "Finish Setup" (fires onboarding-complete)
 *   Step 5: MnemonicStep      — checkbox + "Finish Setup"
 */
export async function walkOnboarding(logPrefix = '[E2E]') {
  const onboardingVisible = (await textExists('Welcome')) ||
    (await textExists('Set up later')) ||
    (await textExists('Continue'));

  if (!onboardingVisible) {
    console.log(`${logPrefix} Onboarding overlay not visible — skipping`);
    await browser.pause(3_000);
    return;
  }

  // Step 0: WelcomeStep
  if (await textExists('Welcome')) {
    const clicked = await clickFirstMatch(['Continue'], 10_000);
    if (clicked) console.log(`${logPrefix} WelcomeStep: clicked "${clicked}"`);
    await browser.pause(2_000);
  }

  // Step 1: LocalAIStep
  {
    const clicked = await clickFirstMatch(['Setup later', 'Use Local Models', 'Continue'], 10_000);
    if (clicked) {
      console.log(`${logPrefix} LocalAIStep: clicked "${clicked}"`);
      await browser.pause(2_000);
      if (clicked === 'Setup later') {
        const cont = await clickFirstMatch(['Continue'], 5_000);
        if (cont) {
          console.log(`${logPrefix} LocalAIStep (skipped): clicked "Continue"`);
          await browser.pause(2_000);
        }
      }
    }
  }

  // Step 2: ScreenPermissionsStep
  {
    const clicked = await clickFirstMatch(['Continue Without Permission', 'Continue'], 10_000);
    if (clicked) {
      console.log(`${logPrefix} ScreenPermissionsStep: clicked "${clicked}"`);
      await browser.pause(2_000);
    }
  }

  // Step 3: ToolsStep
  {
    if (await textExists('Enable Tools')) {
      const clicked = await clickFirstMatch(['Continue'], 10_000);
      if (clicked) {
        console.log(`${logPrefix} ToolsStep: clicked "${clicked}"`);
        await browser.pause(2_000);
      }
    }
  }

  // Step 4: SkillsStep
  {
    if (await textExists('Install Skills')) {
      const clicked = await clickFirstMatch(['Finish Setup'], 10_000);
      if (clicked) {
        console.log(`${logPrefix} SkillsStep: clicked "${clicked}"`);
        await browser.pause(3_000);
      }
    }
  }

  // Step 5: MnemonicStep
  {
    if (await textExists('Your Recovery Phrase')) {
      console.log(`${logPrefix} MnemonicStep: visible`);
      try {
        await browser.execute(() => {
          const checkbox = document.querySelector('input[type="checkbox"]') as HTMLInputElement;
          if (checkbox && !checkbox.checked) checkbox.click();
        });
      } catch (err) {
        console.log(`${logPrefix} MnemonicStep: checkbox failed:`, err);
      }
      await browser.pause(1_000);
      const clicked = await clickFirstMatch(['Finish Setup'], 10_000);
      if (clicked) {
        console.log(`${logPrefix} MnemonicStep: clicked "${clicked}"`);
        await browser.pause(3_000);
      }
    }
  }
}

// ---------------------------------------------------------------------------
// Full login flow
// ---------------------------------------------------------------------------

export async function performFullLogin(token = 'e2e-test-token', logPrefix = '[E2E]') {
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
  console.log(`${logPrefix} Home page confirmed: found "${homeText}"`);
}
