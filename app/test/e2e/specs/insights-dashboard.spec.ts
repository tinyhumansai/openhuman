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
    await navigateViaHash('/intelligence');

    // Tabs / page chrome — Memory is the canonical first view.
    await waitForText('Memory', 15_000);
    expect(await textExists('Memory')).toBe(true);
  });

  it('renders the actionable-items search input (11.2.3) and accepts a query', async () => {
    // The Memory tab mounts an `<input id="actionable-search">` — assert by id
    // so the test cannot false-pass on an unrelated input elsewhere on the page.
    // Real keystroke synthesis via the React onChange path is intentional:
    // there is no shared helper for typing into arbitrary inputs (only
    // clickButton / clickText / clickToggle), and `browser.keys()` is unreliable
    // on tauri-driver, so we follow the established pattern from
    // `command-palette.spec.ts` (event synthesis via `browser.execute`).
    stepLog('typing into #actionable-search');
    const typed = await browser.execute(() => {
      const target = document.querySelector<HTMLInputElement>('#actionable-search');
      if (!target) return false;
      target.focus();
      const setter = Object.getOwnPropertyDescriptor(
        window.HTMLInputElement.prototype,
        'value'
      )?.set;
      setter?.call(target, 'roundtrip canary');
      target.dispatchEvent(new Event('input', { bubbles: true }));
      return target.value === 'roundtrip canary';
    });
    expect(typed).toBe(true);
  });

  it('renders the actionable-source select (11.2.2) with the All Sources option', async () => {
    // 11.2.2 source filtering is a `<select id="actionable-source">` element
    // (not provider chips). Asserting on the id + the canonical first option
    // proves the filter UI mounted without false-positives on stray buttons.
    const filterPresent = await browser.execute(() => {
      const select = document.querySelector<HTMLSelectElement>('#actionable-source');
      if (!select) return false;
      const allOption = Array.from(select.options).find(o => o.value === 'all');
      return Boolean(allOption && /all sources/i.test(allOption.textContent || ''));
    });
    expect(filterPresent).toBe(true);
  });
});
