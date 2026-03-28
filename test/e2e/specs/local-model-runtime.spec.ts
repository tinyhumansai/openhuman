// @ts-nocheck
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { triggerAuthDeepLink } from '../helpers/deep-link-helpers';
import {
  clickText,
  dumpAccessibilityTree,
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { clearRequestLog, getRequestLog, startMockServer, stopMockServer } from '../mock-server';

async function waitForRequest(method, urlFragment, timeout = 15_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const log = getRequestLog();
    const match = log.find(r => r.method === method && r.url.includes(urlFragment));
    if (match) return match;
    await browser.pause(500);
  }
  return undefined;
}

async function waitForTextToDisappear(text, timeout = 10_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (!(await textExists(text))) return true;
    await browser.pause(400);
  }
  return false;
}

async function completeOnboardingIfVisible() {
  if (await textExists('Skip for now')) {
    await clickText('Skip for now', 10_000);
    await waitForTextToDisappear('Skip for now', 8_000);
    await browser.pause(1500);
  }

  if (await textExists('Looks Amazing')) {
    await clickText('Looks Amazing', 10_000);
    await browser.pause(1500);
  } else if (await textExists('Bring It On')) {
    await clickText('Bring It On', 10_000);
    await browser.pause(1500);
  }

  if (await textExists('Got it')) {
    await clickText('Got it', 10_000);
    await browser.pause(1500);
  } else if (await textExists('Continue')) {
    await clickText('Continue', 10_000);
    await browser.pause(1500);
  }

  if (await textExists("Let's Go")) {
    await clickText("Let's Go", 10_000);
  } else if (await textExists("I'm Ready")) {
    await clickText("I'm Ready", 10_000);
  }
}

async function waitForHome(timeout = 20_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await textExists('Message OpenHuman')) return true;
    await browser.pause(700);
  }
  return false;
}

async function waitForAnyText(candidates, timeout = 20_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    for (const t of candidates) {
      if (await textExists(t)) return t;
    }
    await browser.pause(600);
  }
  return null;
}

describe('Local model runtime flow', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    clearRequestLog();
  });

  after(async () => {
    await stopMockServer();
  });

  it('can trigger local model bootstrap from UI and enter active runtime state', async () => {
    await triggerAuthDeepLink('e2e-local-model-token');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);

    const consume = await waitForRequest('POST', '/telegram/login-tokens/');
    expect(consume).toBeDefined();

    await completeOnboardingIfVisible();

    const onHome = await waitForHome(20_000);
    if (!onHome) {
      const tree = await dumpAccessibilityTree();
      console.log('[LocalModelE2E] Home not reached. Tree:\n', tree.slice(0, 4000));
    }
    expect(onHome).toBe(true);

    await waitForText('Local model runtime', 15_000);
    await clickText('Manage', 10_000);

    await waitForText('Runtime Status', 15_000);

    const incompatibleError =
      'Local model runtime is unavailable in this core build. Restart app after updating to the latest build.';
    expect(await textExists(incompatibleError)).toBe(false);

    await clickText('Bootstrap / Resume', 12_000);
    await waitForAnyText(['Triggering...'], 8_000);

    const activeState = await waitForAnyText(['Downloading', 'Loading', 'Ready'], 25_000);
    if (!activeState) {
      const tree = await dumpAccessibilityTree();
      console.log('[LocalModelE2E] No active runtime state seen. Tree:\n', tree.slice(0, 5000));
    }
    expect(activeState).not.toBeNull();
  });
});
