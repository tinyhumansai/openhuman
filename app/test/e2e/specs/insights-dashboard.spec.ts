import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { supportsExecuteScript } from '../helpers/platform';
import { completeOnboardingIfVisible, navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

/**
 * Insights dashboard smoke spec (features 11.1.3 analyze trigger,
 * 11.2.1 memory view, 11.2.2 source filtering, 11.2.3 search).
 *
 * Goal: prove the Brain memory graph route mounts, its graph surface renders,
 * and the memory actions toolbar is available. Backend wiring (real memory
 * population) is asserted in `memory-roundtrip.spec.ts`; this spec focuses on
 * the dashboard surface.
 *
 * Mac2 skipped — Intelligence sidebar mapping not yet exposed to Appium
 * helpers.
 */
function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[InsightsDashboardE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[InsightsDashboardE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

describe('Insights dashboard smoke', () => {
  before(async function beforeSuite() {
    this.timeout(90_000);
    if (!supportsExecuteScript()) {
      stepLog('Skipping suite on Mac2 — Intelligence sidebar not mapped');
      this.skip();
    }

    stepLog('starting mock server');
    await startMockServer();
    stepLog('waiting for app');
    await waitForApp();
    stepLog('triggering auth bypass deep link');
    await triggerAuthDeepLinkBypass('e2e-insights-dashboard');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible('[InsightsDashboardE2E]');
  });

  after(async () => {
    stepLog('stopping mock server');
    await stopMockServer();
  });

  it('mounts Brain and renders the Graph tab', async () => {
    stepLog('navigating to /brain?tab=graph');
    await navigateViaHash('/brain?tab=graph');

    await waitForText('Graph', 15_000);
    expect(await textExists('Graph')).toBe(true);
  });

  it('renders the memory graph surface (11.2.3)', async () => {
    stepLog('checking for memory graph testid');
    const deadline = Date.now() + 10_000;
    let present = false;
    while (Date.now() < deadline) {
      present = (await browser.execute(
        () =>
          document.querySelector('[data-testid="memory-graph-svg"]') !== null ||
          document.querySelector('[data-testid="memory-graph-empty"]') !== null
      )) as boolean;
      if (present) break;
      await browser.pause(500);
    }
    expect(present).toBe(true);
  });

  it('renders the memory actions toolbar (11.2.2)', async () => {
    // The memory actions bar (wipe / reset / refresh / build buttons) should
    // be mounted above the graph, confirming the tab content fully rendered.
    const actionsPresent = await browser.execute(
      () => document.querySelector('[data-testid="memory-actions"]') !== null
    );
    expect(actionsPresent).toBe(true);
  });
});
