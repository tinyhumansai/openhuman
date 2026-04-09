// @ts-nocheck
/**
 * E2E test: Screen Intelligence (Built-in Skill — accessed from Skills tab)
 *
 * Covers:
 *   9.1.1 — Navigate to Screen Intelligence settings via Skills page built-in card
 *   9.1.2 — Verify permissions section renders (Screen Recording, Accessibility, Input Monitoring)
 *   9.1.3 — Verify Screen Intelligence Policy section renders with toggles and config
 *   9.1.4 — accessibility_status RPC returns platform status and permissions
 *   9.1.5 — screen_intelligence_capture_test RPC fires and returns success or platform error
 *
 * The mock server runs on http://127.0.0.1:18473
 */
import { browser, expect } from '@wdio/globals';

import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  clickText,
  dumpAccessibilityTree,
  hasAppChrome,
  textExists,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { supportsExecuteScript } from '../helpers/platform';
import {
  completeOnboardingIfVisible,
  dismissLocalAISnackbarIfVisible,
  navigateViaHash,
  waitForHomePage,
} from '../helpers/shared-flows';
import { clearRequestLog, startMockServer, stopMockServer } from '../mock-server';

const LOG_PREFIX = '[ScreenIntelligenceE2E]';

function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`${LOG_PREFIX}[${stamp}] ${message}`);
    return;
  }
  console.log(`${LOG_PREFIX}[${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

async function waitForAnyText(candidates: string[], timeout = 15_000): Promise<string | null> {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    for (const t of candidates) {
      if (await textExists(t)) return t;
    }
    await browser.pause(500);
  }
  return null;
}

describe('Screen Intelligence', () => {
  before(async () => {
    stepLog('Starting Screen Intelligence E2E');
    await startMockServer();
    await waitForApp();
    clearRequestLog();
  });

  after(async () => {
    await stopMockServer();
  });

  // ── Auth + reach app shell ──────────────────────────────────────────────

  it('authenticates and reaches the app shell', async () => {
    await triggerAuthDeepLinkBypass('e2e-screen-intelligence-user');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible(LOG_PREFIX);
    expect(await hasAppChrome()).toBe(true);

    const home = await waitForHomePage(15_000);
    if (!home) {
      const tree = await dumpAccessibilityTree();
      stepLog('Home page not reached', { tree: tree.slice(0, 4000) });
    }
    expect(home).not.toBeNull();
  });

  // ── 9.1.1 Navigate to Screen Intelligence via Skills built-in card ──────

  it('navigates to Screen Intelligence from the Skills page built-in card', async () => {
    await dismissLocalAISnackbarIfVisible(LOG_PREFIX);
    await navigateViaHash('/skills');
    await browser.pause(2_000);

    const hasBuiltIn = await waitForAnyText(['Built-in Skills', 'Screen Intelligence'], 10_000);
    stepLog('Skills page built-in section', { found: hasBuiltIn });
    expect(hasBuiltIn).not.toBeNull();

    // Click the Screen Intelligence card → /settings/screen-intelligence
    await clickText('Screen Intelligence', 10_000);
    await browser.pause(2_000);

    if (supportsExecuteScript()) {
      const currentHash = await browser.execute(() => window.location.hash);
      stepLog('After clicking Screen Intelligence card', { currentHash });
      expect(currentHash).toContain('screen-intelligence');
    }
  });

  // ── 9.1.2 Verify Permissions section ────────────────────────────────────

  it('shows the Permissions section with platform permission badges', async () => {
    const alreadyOnPage = await textExists('Screen Intelligence Policy');
    if (!alreadyOnPage) {
      await navigateViaHash('/settings/screen-intelligence');
      await browser.pause(2_000);
    }

    const hasPage = await waitForAnyText(
      ['Screen Intelligence', 'Permissions', 'Screen Intelligence Policy'],
      15_000
    );
    if (!hasPage) {
      const tree = await dumpAccessibilityTree();
      stepLog('Screen Intelligence page headings missing', { tree: tree.slice(0, 4000) });
    }
    expect(hasPage).not.toBeNull();

    const permFound = await waitForAnyText(
      ['Screen Recording', 'Accessibility', 'Input Monitoring', 'Permissions'],
      10_000
    );
    stepLog('Permissions section', { found: permFound });
    expect(permFound).not.toBeNull();
  });

  // ── 9.1.3 Verify Screen Intelligence Policy config ─────────────────────

  it('shows the Screen Intelligence Policy section with configuration options', async () => {
    const alreadyOnPage = await textExists('Screen Intelligence Policy');
    if (!alreadyOnPage) {
      await navigateViaHash('/settings/screen-intelligence');
      await browser.pause(2_000);
    }

    const hasPolicy = await textExists('Screen Intelligence Policy');
    stepLog('Policy section visible', { hasPolicy });
    expect(hasPolicy).toBe(true);

    // Look for config labels visible without scrolling
    const configLabels = [
      'Enabled',
      'Mode',
      'Screen Monitoring',
      'Device Control',
      'Predictive Input',
    ];
    const foundLabels: string[] = [];
    for (const label of configLabels) {
      if (await textExists(label)) foundLabels.push(label);
    }
    stepLog('Config labels found', { foundLabels });
    expect(foundLabels.length).toBeGreaterThanOrEqual(1);
  });

  // ── 9.1.4 screen_intelligence_status RPC ─────────────────────────────────

  it('screen_intelligence_status RPC returns platform status and permissions', async () => {
    const result = await callOpenhumanRpc('openhuman.screen_intelligence_status', {});
    stepLog('screen_intelligence_status RPC raw', JSON.stringify(result, null, 2));

    expect(result.ok).toBe(true);

    // The result may be directly the struct or wrapped in { result, logs }
    const raw = result.result;
    const data = raw?.result ?? raw; // handle { result: {...}, logs: [...] } wrapper
    expect(data).toBeDefined();

    expect(typeof data.platform_supported).toBe('boolean');
    expect(data.permissions).toBeDefined();
    expect(data.session).toBeDefined();

    stepLog('screen_intelligence_status details', {
      platform_supported: data.platform_supported,
      session_active: data.session?.active,
      config_enabled: data.config?.enabled,
      permissions: data.permissions,
    });
  });

  // ── 9.1.5 screen_intelligence_capture_test RPC ──────────────────────────

  it('capture test RPC fires and returns success or platform error', async () => {
    const result = await callOpenhumanRpc('openhuman.screen_intelligence_capture_test', {});
    stepLog('capture_test RPC raw', JSON.stringify(result, null, 2));

    expect(result.ok).toBe(true);

    const raw = result.result;
    const data = raw?.result ?? raw; // handle { result: {...}, logs: [...] } wrapper
    expect(data).toBeDefined();

    if (data.ok === true) {
      stepLog('Capture succeeded', {
        capture_mode: data.capture_mode,
        timing_ms: data.timing_ms,
        bytes_estimate: data.bytes_estimate,
        has_image: !!data.image_ref,
      });
      expect(typeof data.capture_mode).toBe('string');
      expect(typeof data.timing_ms).toBe('number');
    } else {
      // Capture failed — expected in E2E (no screen recording permission)
      stepLog('Capture failed (expected in E2E)', {
        ok: data.ok,
        error: data.error,
        capture_mode: data.capture_mode,
      });
      // ok should be explicitly false
      expect(data.ok).toBe(false);
      // capture_mode or error should be present
      const hasDetail =
        (typeof data.error === 'string' && data.error.length > 0) ||
        typeof data.capture_mode === 'string';
      expect(hasDetail).toBe(true);
    }
  });
});
