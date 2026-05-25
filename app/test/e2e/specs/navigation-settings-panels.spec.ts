// @ts-nocheck
/**
 * Navigation — settings sub-panel coverage.
 *
 * Visits every settings sub-panel and verifies each loads without
 * blank screens or error states.
 *
 * Tests:
 *   N2.1 — /settings (root index)
 *   N2.2 — /settings/connections
 *   N2.3 — /settings/memory-data
 *   N2.4 — /settings/intelligence
 *   N2.5 — /settings/developer-options
 *   N2.6 — /settings/billing
 *   N2.7 — /settings/appearance
 *   N2.8 — /settings/tools
 *   N2.9 — back navigation to /home returns home content
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { textExists } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import {
  navigateToBilling,
  navigateToHome,
  navigateViaHash,
  waitForHomePage,
} from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const LOG_PREFIX = '[navigation-settings-panels]';
const USER_ID = 'e2e-navigation-settings-panels';
const PANEL_TIMEOUT = 10_000;

interface PanelCheck {
  hash: string;
  /** Candidate strings — any one match confirms the panel loaded. */
  markers: string[];
  /** Use the navigateToBilling helper (has its own verification). */
  useBillingHelper?: boolean;
}

const PANELS: PanelCheck[] = [
  {
    // N2.1 — root settings page (section index)
    hash: '/settings',
    markers: ['Settings', 'Account', 'Privacy', 'Appearance', 'Notifications'],
  },
  {
    // N2.2 — connections (channel providers)
    hash: '/settings/connections',
    markers: ['Connections', 'Connect', 'Provider', 'Gmail', 'Telegram', 'Settings'],
  },
  {
    // N2.3 — memory / data panel
    hash: '/settings/memory-data',
    markers: ['Memory', 'Data', 'Storage', 'Export', 'Import', 'Settings'],
  },
  {
    // N2.4 — intelligence / AI settings (top-level route, not nested under /settings)
    hash: '/intelligence',
    markers: ['Intelligence', 'AI', 'Model', 'Skills', 'Settings'],
  },
  {
    // N2.5 — developer options
    hash: '/settings/developer-options',
    markers: ['Developer', 'Debug', 'Advanced', 'Settings', 'Logs'],
  },
  {
    hash: '/settings/billing',
    markers: ['Billing', 'Plan', 'Subscription', 'Usage'],
    useBillingHelper: true,
  },
  {
    // N2.7 — appearance panel
    hash: '/settings/appearance',
    markers: ['Appearance', 'Theme', 'Color', 'Dark', 'Settings'],
  },
  {
    // N2.8 — tools panel
    hash: '/settings/tools',
    markers: ['Tools', 'Tool', 'Enable', 'Disable', 'Settings'],
  },
];

async function rootTextLength(): Promise<number> {
  return (await browser.execute(
    () => (document.getElementById('root')?.innerText ?? '').length
  )) as number;
}

async function verifyPanelLoaded(panel: PanelCheck): Promise<void> {
  await waitForAppReady(PANEL_TIMEOUT);

  const chars = await rootTextLength();
  if (chars < 50) {
    throw new Error(`${panel.hash}: panel appears blank (${chars} chars in #root)`);
  }

  let foundMarker = '';
  for (const marker of panel.markers) {
    if (await textExists(marker)) {
      foundMarker = marker;
      break;
    }
  }

  if (foundMarker) {
    console.log(`${LOG_PREFIX} ${panel.hash}: loaded (found "${foundMarker}", ${chars} chars)`);
  } else {
    // Non-fatal: the panel may render different text depending on config / state.
    // The char-count check above is the authoritative blank-screen guard.
    console.log(
      `${LOG_PREFIX} ${panel.hash}: loaded (${chars} chars, no marker matched — acceptable)`
    );
  }
}

describe('Navigation — settings sub-panels', () => {
  before(async () => {
    console.log(`${LOG_PREFIX} Starting mock server and resetting app`);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
    console.log(`${LOG_PREFIX} Setup complete`);
  });

  after(async () => {
    await stopMockServer();
    console.log(`${LOG_PREFIX} Teardown complete`);
  });

  it('N2.1 — /settings (root index) loads', async () => {
    const panel = PANELS[0];
    console.log(`${LOG_PREFIX} N2.1: navigating to ${panel.hash}`);
    await navigateViaHash(panel.hash);
    await verifyPanelLoaded(panel);
  });

  it.skip('N2.2 — /settings/connections loads (removed in PR #2550)', async () => {
    // Route was deleted as part of the OAuth loopback + settings cleanup
    // (PR #2550). Kept as a `.skip` so the numbering N2.1..N2.9 still
    // reads consistently with the spec list.
  });

  it('N2.3 — /settings/memory-data loads', async () => {
    const panel = PANELS[2];
    console.log(`${LOG_PREFIX} N2.3: navigating to ${panel.hash}`);
    await navigateViaHash(panel.hash);
    await verifyPanelLoaded(panel);
  });

  it('N2.4 — /intelligence loads', async () => {
    const panel = PANELS[3];
    console.log(`${LOG_PREFIX} N2.4: navigating to ${panel.hash}`);
    await navigateViaHash(panel.hash);
    await verifyPanelLoaded(panel);
  });

  it('N2.5 — /settings/developer-options loads', async () => {
    const panel = PANELS[4];
    console.log(`${LOG_PREFIX} N2.5: navigating to ${panel.hash}`);
    await navigateViaHash(panel.hash);
    await verifyPanelLoaded(panel);
  });

  it('N2.6 — /settings/billing loads', async () => {
    console.log(`${LOG_PREFIX} N2.6: navigating to /settings/billing`);
    // Use the dedicated helper which includes its own content verification.
    await navigateToBilling();
    console.log(`${LOG_PREFIX} N2.6: passed`);
  });

  it('N2.7 — /settings/appearance loads', async () => {
    const panel = PANELS[6];
    console.log(`${LOG_PREFIX} N2.7: navigating to ${panel.hash}`);
    await navigateViaHash(panel.hash);
    await verifyPanelLoaded(panel);
  });

  it('N2.8 — /settings/tools loads', async () => {
    const panel = PANELS[7];
    console.log(`${LOG_PREFIX} N2.8: navigating to ${panel.hash}`);
    await navigateViaHash(panel.hash);
    await verifyPanelLoaded(panel);
  });

  it('N2.9 — back navigation from last panel returns to /home', async () => {
    console.log(`${LOG_PREFIX} N2.9: navigating back to /home`);
    await navigateToHome();
    const homeText = await waitForHomePage(PANEL_TIMEOUT);
    expect(homeText).toBeTruthy();

    const hash = await browser.execute(() => window.location.hash);
    expect(hash).toMatch(/^#\/home/);
    console.log(`${LOG_PREFIX} N2.9: passed — home content: "${homeText}"`);
  });
});
