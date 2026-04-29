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
 * Autocomplete settings panel smoke spec — narrow scope.
 *
 * What this spec proves: the AutocompletePanel mounts under /settings,
 * the skill-status pill renders one of the canonical labels surfaced by
 * `useAutocompleteSkillStatus`, and the matching CTA renders. That is
 * the entire claim — this spec does NOT exercise:
 *   - 5.2.1 inline suggestion generation (requires real keystrokes inside
 *     a third-party text field + macOS Accessibility + Input Monitoring
 *     TCC grants — see manual smoke checklist #971)
 *   - 5.2.2 debounce timing (covered by the Vitest hook test in
 *     `app/src/features/autocomplete/__tests__/useAutocompleteSkillStatus.test.tsx`
 *     for the status surface; debounce of the engine itself is a Rust
 *     unit test concern)
 *   - 5.2.3 acceptance trigger (manual smoke + Rust unit)
 *
 * The coverage matrix downgrades 5.2.1 / 5.2.3 to 🟡 to reflect this.
 *
 * Mac2 skipped — Settings sidebar label mapping not yet exposed to Appium.
 */
function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[AutocompleteFlowE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[AutocompleteFlowE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

describe('Autocomplete settings panel smoke', () => {
  before(async function beforeSuite() {
    if (!supportsExecuteScript()) {
      stepLog('Skipping suite on Mac2 — Settings sidebar not mapped');
      this.skip();
    }

    stepLog('starting mock server');
    await startMockServer();
    stepLog('waiting for app');
    await waitForApp();
    stepLog('triggering auth bypass deep link');
    await triggerAuthDeepLinkBypass('e2e-autocomplete-flow');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible('[AutocompleteFlowE2E]');
  });

  after(async () => {
    stepLog('stopping mock server');
    await stopMockServer();
  });

  it('mounts the autocomplete settings panel and renders status', async () => {
    stepLog('navigating to /settings/autocomplete');
    await navigateViaHash('/settings/autocomplete');

    // Panel chrome — at least one of the skill-status labels rendered by
    // useAutocompleteSkillStatus must show. Status text is one of:
    // Active / Offline / Error / Unsupported.
    await waitForText('Auto', 15_000);
    const statusVisible =
      (await textExists('Active')) ||
      (await textExists('Offline')) ||
      (await textExists('Error')) ||
      (await textExists('Unsupported'));
    expect(statusVisible).toBe(true);
  });

  it('renders an Enable / Manage / Retry CTA driven by skill status', async () => {
    // Re-establish route state so this case is runnable in isolation; do not
    // depend on the previous `it` having navigated to /settings/autocomplete.
    stepLog('navigating to /settings/autocomplete (independent setup)');
    await navigateViaHash('/settings/autocomplete');
    await waitForText('Auto', 15_000);

    const ctaVisible =
      (await textExists('Enable')) ||
      (await textExists('Manage')) ||
      (await textExists('Retry')) ||
      (await textExists('Details'));
    expect(ctaVisible).toBe(true);
  });
});
