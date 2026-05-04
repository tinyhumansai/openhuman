// @ts-nocheck
/**
 * Canonical "agent review" E2E flow.
 *
 * Goal: one deterministic, mock-backed path through onboarding + the privacy
 * settings panel that produces a readable artifact trail on disk so coding
 * agents can:
 *   - launch the app into a known state,
 *   - navigate via automation,
 *   - inspect screenshots + page source at each checkpoint,
 *   - inspect mock backend request evidence.
 *
 * See docs/AGENT-OBSERVABILITY.md for how artifacts are laid out.
 *
 * This spec intentionally keeps assertions loose: its primary contract is
 * "the flow reaches each checkpoint and captures artifacts", not a strict
 * UI assertion — we already have login-flow.spec.ts for that.
 */
import { waitForApp, waitForAppReady, waitForAuthBootstrap } from '../helpers/app-helpers';
import { captureCheckpoint, getArtifactDir, saveMockRequestLog } from '../helpers/artifacts';
import { triggerAuthDeepLink } from '../helpers/deep-link-helpers';
import {
  clickText,
  textExists,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import {
  clearRequestLog,
  getRequestLog,
  resetMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

async function tryClick(text: string, timeout = 5_000): Promise<boolean> {
  if (!(await textExists(text))) return false;
  try {
    await clickText(text, timeout);
    return true;
  } catch {
    return false;
  }
}

async function waitForAny(texts: string[], timeout = 10_000): Promise<string | null> {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    for (const t of texts) {
      if (await textExists(t)) return t;
    }
    await browser.pause(500);
  }
  return null;
}

describe('Agent review — canonical onboarding + privacy flow', () => {
  before(async () => {
    // Force label so the run dir is predictable: "<ts>-agent-review".
    process.env.E2E_ARTIFACT_LABEL = process.env.E2E_ARTIFACT_LABEL || 'agent-review';
    getArtifactDir();

    await startMockServer();
    await waitForApp();
    clearRequestLog();
  });

  after(async () => {
    resetMockBehavior();
    await stopMockServer();

    console.log(`[agent-review] artifacts: ${getArtifactDir()}`);
  });

  it('01 launches and reaches welcome', async () => {
    await triggerAuthDeepLink('e2e-agent-review-token');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await waitForAuthBootstrap(15_000);

    await waitForAny(['Welcome', 'Continue', "Let's Start", 'Home'], 15_000);
    await captureCheckpoint('welcome');
    saveMockRequestLog('after-welcome', getRequestLog());
  });

  it('02 advances past welcome step', async () => {
    const clicked =
      (await tryClick("Let's Start")) || (await tryClick('Continue')) || (await tryClick('Skip'));

    console.log(`[agent-review] welcome advance clicked=${clicked}`);
    await browser.pause(2_000);
    await captureCheckpoint('post-welcome');
  });

  it('03 walks remaining onboarding steps or lands on home', async () => {
    // Referral
    if (await textExists('Referral code')) {
      await tryClick('Skip for now');
      await browser.pause(1_500);
    }
    // Skills
    if (await textExists('Connect Gmail')) {
      await tryClick('Skip for Now');
      await browser.pause(2_000);
    }
    // Context gathering (may auto-skip)
    if (await textExists('Context')) {
      await tryClick('Continue');
      await browser.pause(1_500);
    }

    await waitForAny(['Home', 'Skills', 'Conversations', 'Settings'], 15_000);
    await captureCheckpoint('post-onboarding');
    saveMockRequestLog('after-onboarding', getRequestLog());
  });

  it('04 opens settings privacy panel', async () => {
    // Navigate via hash route — works on tauri-driver and Mac2 WebView.
    try {
      await browser.execute(() => {
        window.location.hash = '#/settings/privacy';
      });
    } catch {
      // Non-fatal: if hash nav is unavailable, we still capture what we see.
    }
    await browser.pause(2_000);
    await waitForAny(['Privacy', 'Analytics'], 10_000);
    await captureCheckpoint('privacy-panel');
    saveMockRequestLog('after-privacy', getRequestLog());
  });
});
