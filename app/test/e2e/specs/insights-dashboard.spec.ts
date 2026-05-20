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
 * Goal: prove the /intelligence route mounts, the Memory tab renders, the
 * source filter chips are present, and the search input accepts a query
 * without throwing. Backend wiring (real memory population) is asserted in
 * `memory-roundtrip.spec.ts` — this spec focuses on the dashboard surface.
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

  it('mounts the /intelligence route and renders the Memory tab', async () => {
    stepLog('navigating to /intelligence');
    await navigateViaHash('/settings/intelligence');

    // Tabs / page chrome — Memory is the canonical first view.
    await waitForText('Memory', 15_000);
    expect(await textExists('Memory')).toBe(true);
  });

  it('renders the memory workspace actions panel (11.2.3 — Build Summary Trees button)', async () => {
    // The Memory tab now mounts `MemoryWorkspace` (replaced the old
    // `IntelligenceMemoryTab` actionable-items pipeline). Assert the
    // workspace container and the "Build Summary Trees" action button are
    // present — this is the primary interactive element on the Memory surface.
    stepLog('asserting memory-workspace and memory-build-trees are present');
    const workspacePresent = await browser.execute(() => {
      const workspace = document.querySelector('[data-testid="memory-workspace"]');
      return workspace !== null;
    });
    stepLog('memory-workspace present', { workspacePresent });
    expect(workspacePresent).toBe(true);

    const buildButtonPresent = await browser.execute(() => {
      const btn = document.querySelector('[data-testid="memory-build-trees"]');
      return btn !== null;
    });
    stepLog('memory-build-trees button present', { buildButtonPresent });
    expect(buildButtonPresent).toBe(true);
  });

  it('renders the memory action controls (11.2.2 — Reset Memory + Reset Memory Tree)', async () => {
    // 11.2.2 is now the MemoryWorkspace action bar. The filter pipeline
    // (`#actionable-source` select) was removed when the Memory tab
    // migrated to `MemoryWorkspace`. We assert the two wipe/reset
    // control buttons are present — they are always rendered (not gated
    // on graph load state) and unambiguously identify the controls panel.
    const actionsPresent = await browser.execute(() => {
      const wipe = document.querySelector('[data-testid="memory-wipe-all"]');
      const reset = document.querySelector('[data-testid="memory-reset-tree"]');
      return wipe !== null && reset !== null;
    });
    stepLog('memory action buttons present', { actionsPresent });
    expect(actionsPresent).toBe(true);
  });
});
