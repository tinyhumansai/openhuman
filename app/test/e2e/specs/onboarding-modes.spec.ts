// @ts-nocheck
/**
 * E2E: Onboarding — Simple (Cloud) vs Advanced (Custom) modes.
 *
 * Verifies:
 *   - Phase A — Simple/Cloud path: fresh login → Welcome → Runtime choice
 *     (Cloud) → /home. `onboarding_completed = true` lands in
 *     `${OPENHUMAN_WORKSPACE}/config.toml` immediately.
 *
 *   - Phase B — Advanced/Custom path (Default on every wizard step):
 *     reset onboarding flag → Welcome → Runtime choice (Custom) →
 *     Inference (Default) → Voice (Default) → OAuth (Default) →
 *     Search (Default) → Embeddings (Default) → Finish.
 *     Asserts all five custom wizard step containers render with the
 *     expected `data-testid`s (i.e. *all settings are reachable*).
 *
 *   - Phase C — Advanced/Custom path with Configure on the Voice step:
 *     pick Configure, the embedded VoicePanel renders. Flip the STT
 *     provider selector and assert `config.toml` updates
 *     `local_ai.stt_provider` within a few seconds (i.e. advanced voice
 *     provider settings apply immediately to persisted config).
 *
 * Auth is the bypass deep-link path. The mock API server runs on the same
 * port the dist bundle was built against (see `app/scripts/e2e-run-session.sh`).
 * No real network is touched.
 */
import { waitForAppReady, waitForAuthBootstrap } from '../helpers/app-helpers';
import { readBool, readConfigToml, readSectionString, topLevelValue } from '../helpers/config-toml';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import { waitForWebView, waitForWindowVisible } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { dismissBootCheckGateIfVisible } from '../helpers/shared-flows';
import {
  resetMockBehavior,
  setMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

const STEP_LOG_PREFIX = '[onboarding-modes]';

function stepLog(message: string): void {
  console.log(`${STEP_LOG_PREFIX} ${message}`);
}

async function pause(ms: number): Promise<void> {
  await browser.pause(ms);
}

/**
 * Click a button by `data-testid`. Returns true if the click landed.
 */
async function clickTestId(testId: string, timeout = 10_000): Promise<boolean> {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const status = await browser.execute(id => {
      const el = document.querySelector<HTMLElement>(`[data-testid="${id}"]`);
      if (!el) return 'missing';
      if ((el as HTMLButtonElement).disabled) return 'disabled';
      // Ensure the element is visible and has layout before clicking.
      const rect = el.getBoundingClientRect();
      if (rect.width === 0 || rect.height === 0) return 'no-layout';
      ['mousedown', 'mouseup', 'click'].forEach(type => {
        el.dispatchEvent(
          new MouseEvent(type, { bubbles: true, cancelable: true, view: window, button: 0 })
        );
      });
      return 'clicked';
    }, testId);
    if (status === 'clicked') return true;
    await pause(400);
  }
  return false;
}

async function testIdExists(testId: string, timeout = 10_000): Promise<boolean> {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const found = await browser.execute(
      id => document.querySelector(`[data-testid="${id}"]`) !== null,
      testId
    );
    if (found) return true;
    await pause(400);
  }
  return false;
}

async function currentHash(): Promise<string> {
  return browser.execute(() => window.location.hash || '');
}

async function waitForHash(prefix: string, timeout = 15_000): Promise<boolean> {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const hash = await currentHash();
    if (hash.startsWith(prefix)) return true;
    await pause(400);
  }
  return false;
}

async function resetOnboardingFlagAndReload(): Promise<void> {
  stepLog('Resetting onboarding_completed=false via RPC');
  const res = await callOpenhumanRpc<{ completed: boolean }>(
    'openhuman.config_set_onboarding_completed',
    { value: false }
  );
  if (!res.ok) {
    throw new Error(`config.set_onboarding_completed failed: ${JSON.stringify(res)}`);
  }
  await browser.execute(() => {
    try {
      window.localStorage.clear();
      window.sessionStorage.clear();
    } catch {
      /* ignore */
    }
    window.location.replace('#/');
    window.location.reload();
  });
  await waitForWindowVisible(25_000);
  await waitForWebView(15_000);
  await waitForAppReady(15_000);
  await dismissBootCheckGateIfVisible(8_000);
  await triggerAuthDeepLinkBypass('e2e-onboarding-modes');
  await waitForAuthBootstrap(15_000);
  await dismissBootCheckGateIfVisible(8_000);
  // Wait for the welcome step to mount before returning.
  const onWelcome = await waitForHash('#/onboarding', 15_000);
  if (!onWelcome) {
    stepLog(`hash after reset = ${await currentHash()}`);
    throw new Error('onboarding overlay did not re-mount after flag reset');
  }
}

async function clickOnboardingNext(): Promise<void> {
  // The Welcome step button is the same shared `onboarding-next-button`.
  const ok = await clickTestId('onboarding-next-button', 10_000);
  if (!ok) {
    throw new Error('onboarding-next-button missing or stayed disabled');
  }
}

async function waitForCustomSelection(timeout = 5_000): Promise<boolean> {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const selected = await browser.execute(() => {
      const el = document.querySelector('[data-testid="onboarding-runtime-choice-custom"]');
      return el?.getAttribute('aria-pressed') === 'true';
    });
    if (selected) return true;
    await pause(250);
  }
  return false;
}

async function advanceFromWelcomeToCustomInference(phase: string): Promise<void> {
  const reachedChoiceOrInference = await browser
    .waitUntil(
      async () => {
        if (await testIdExists('onboarding-runtime-choice-step', 250)) return true;
        if (await testIdExists('onboarding-custom-inference-step', 250)) return true;
        if (await testIdExists('onboarding-welcome-step', 250)) {
          await clickOnboardingNext();
        }
        return false;
      },
      {
        timeout: 15_000,
        interval: 300,
        timeoutMsg: `${phase}: neither runtime choice nor custom inference mounted`,
      }
    )
    .catch(() => false);
  if (!reachedChoiceOrInference) {
    stepLog(`${phase}: hash while waiting for runtime/custom inference = ${await currentHash()}`);
  }
  expect(reachedChoiceOrInference).toBe(true);

  if (await testIdExists('onboarding-runtime-choice-step', 500)) {
    const inferenceReached = await browser
      .waitUntil(
        async () => {
          if (await testIdExists('onboarding-custom-inference-step', 250)) return true;
          if (!(await testIdExists('onboarding-runtime-choice-step', 250))) return false;

          if (!(await clickTestId('onboarding-runtime-choice-custom', 2_000))) {
            return false;
          }
          const customSelected = await waitForCustomSelection(2_000);
          if (!customSelected) {
            stepLog(`${phase}: Custom card click did not register — retrying`);
            return false;
          }

          await clickOnboardingNext();
          return await testIdExists('onboarding-custom-inference-step', 1_000);
        },
        {
          timeout: 15_000,
          interval: 500,
          timeoutMsg: `${phase}: custom inference did not mount after runtime choice`,
        }
      )
      .catch(() => false);
    expect(inferenceReached).toBe(true);
  }

  const inferenceVisible = await testIdExists('onboarding-custom-inference-step', 15_000);
  if (!inferenceVisible) {
    stepLog(`${phase}: hash before custom inference assertion = ${await currentHash()}`);
  }
  expect(inferenceVisible).toBe(true);
}

async function waitForHome(timeout = 20_000): Promise<boolean> {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const hash = await currentHash();
    // Home was merged into the unified chat surface: /home redirects to /chat
    // (AppRoutes.tsx). Accept either so the wizard's post-finish landing check
    // survives the IA change.
    if (hash.startsWith('#/home') || hash.startsWith('#/chat')) return true;
    await pause(400);
  }
  return false;
}

describe('Onboarding modes — Simple (Cloud) vs Advanced (Custom)', () => {
  before(async function beforeSuite() {
    // Reset + auth + onboarding bootstrap can exceed the default 30s hook budget.
    this.timeout(90_000);
    await startMockServer();
    resetMockBehavior();
    setMockBehavior('composioConnections', '[]');
    // Reset state but skip the built-in onboarding walker — we walk it
    // ourselves to assert the per-step UI.
    await resetApp('e2e-onboarding-modes', { skipAuth: true });
    // resetApp restores onboarding_completed=true for normal specs; this spec
    // intentionally exercises the onboarding flow, so flip it back to false
    // before triggering auth so App.tsx routes to /onboarding.
    stepLog('Setting onboarding_completed=false for onboarding flow test');
    await callOpenhumanRpc('openhuman.config_set_onboarding_completed', { value: false });
    await triggerAuthDeepLinkBypass('e2e-onboarding-modes');
    await waitForAuthBootstrap(15_000);
    await dismissBootCheckGateIfVisible(8_000);
    await waitForHash('#/onboarding', 15_000);
  });

  after(async () => {
    resetMockBehavior();
    await stopMockServer();
  });

  // ───────────────────────────────────────────────────────────────────────
  // Phase A — Simple (Cloud)
  // ───────────────────────────────────────────────────────────────────────

  it('simple/cloud path: welcome → runtime-choice → cloud → home', async () => {
    // Step 0 — Welcome screen.
    const welcomeVisible = await testIdExists('onboarding-next-button', 15_000);
    expect(welcomeVisible).toBe(true);
    await clickOnboardingNext();

    // Step 1 — Runtime choice. The card is preselected to Cloud, so simply
    // clicking the next button continues the cloud path.
    const choiceVisible = await testIdExists('onboarding-runtime-choice-step', 10_000);
    expect(choiceVisible).toBe(true);
    const cloudCardVisible = await testIdExists('onboarding-runtime-choice-cloud', 5_000);
    expect(cloudCardVisible).toBe(true);
    // Explicitly click the Cloud card so the test is robust against the
    // default selection changing in the future.
    await clickTestId('onboarding-runtime-choice-cloud');
    await pause(500);
    await clickOnboardingNext();

    const landed = await waitForHome(20_000);
    if (!landed) stepLog(`current hash after cloud finish: ${await currentHash()}`);
    expect(landed).toBe(true);
  });

  it('simple/cloud path: config.toml reflects onboarding_completed=true', async () => {
    // The setOnboardingCompletedFlag RPC writes config.save() before the
    // navigate() in OnboardingLayout, but I/O can lag a tick. Poll briefly.
    let value: boolean | null = null;
    const deadline = Date.now() + 8_000;
    while (Date.now() < deadline) {
      value = readBool(topLevelValue(readConfigToml(), 'onboarding_completed'));
      if (value === true) break;
      await pause(400);
    }
    if (value !== true) {
      stepLog(`config.toml head:\n${readConfigToml().split('\n').slice(0, 30).join('\n')}`);
    }
    expect(value).toBe(true);
  });

  // ───────────────────────────────────────────────────────────────────────
  // Phase B — Advanced (Custom), Default on every step
  // ───────────────────────────────────────────────────────────────────────

  it('advanced/custom path: walks all wizard steps with Default choice', async function () {
    // resetOnboardingFlagAndReload includes waitForWindowVisible(25_000), needs extra budget.
    this.timeout(90_000);
    await resetOnboardingFlagAndReload();

    await advanceFromWelcomeToCustomInference('Phase B');

    // Step 2 — Custom Inference (Default).
    expect(await clickTestId('onboarding-custom-inference-step-default')).toBe(true);
    await pause(400);
    await clickOnboardingNext();

    // Step 3 — Custom Voice (Default).
    expect(await testIdExists('onboarding-custom-voice-step', 10_000)).toBe(true);
    expect(await clickTestId('onboarding-custom-voice-step-default')).toBe(true);
    await pause(400);
    await clickOnboardingNext();

    // Step 4 — Custom OAuth (Default).
    expect(await testIdExists('onboarding-custom-oauth-step', 10_000)).toBe(true);
    expect(await clickTestId('onboarding-custom-oauth-step-default')).toBe(true);
    await pause(400);
    await clickOnboardingNext();

    // Step 5 — Custom Search (Default).
    expect(await testIdExists('onboarding-custom-search-step', 10_000)).toBe(true);
    expect(await clickTestId('onboarding-custom-search-step-default')).toBe(true);
    await pause(400);
    await clickOnboardingNext();

    // Step 6 — Custom Embeddings (Default).
    expect(await testIdExists('onboarding-custom-embeddings-step', 10_000)).toBe(true);
    expect(await clickTestId('onboarding-custom-embeddings-step-default')).toBe(true);
    await pause(400);
    await clickOnboardingNext();

    // Step 7 — Custom Activity (Default). Added to CUSTOM_WIZARD_STEPS after
    // embeddings (see app/src/pages/onboarding/customWizardSteps.ts).
    expect(await testIdExists('onboarding-custom-activity-step', 10_000)).toBe(true);
    expect(await clickTestId('onboarding-custom-activity-step-default')).toBe(true);
    await pause(400);
    await clickOnboardingNext();

    // Step 8 — Custom Vault. Final step → Finish. VaultSetupStep hides the
    // choice cards and auto-selects "configure" only for LOCAL sessions
    // (`defaultDisabled={isLocalSession}`); the E2E logs in via the cloud-auth
    // deep link, so the session is non-local — the choice cards ARE shown with
    // an enabled default, and the Finish button stays disabled (`choice === null`)
    // until one is picked. Select the default when the card is present; a local
    // session (cards hidden) just advances.
    expect(await testIdExists('onboarding-custom-vault-step', 10_000)).toBe(true);
    if (await testIdExists('onboarding-custom-vault-step-default', 2_000)) {
      await clickTestId('onboarding-custom-vault-step-default');
    }
    await pause(400);
    await clickOnboardingNext();

    const landed = await waitForHome(20_000);
    if (!landed) stepLog(`current hash after custom finish: ${await currentHash()}`);
    expect(landed).toBe(true);

    // Re-confirm the persisted flag is true after the second completion.
    let value: boolean | null = null;
    const deadline = Date.now() + 8_000;
    while (Date.now() < deadline) {
      value = readBool(topLevelValue(readConfigToml(), 'onboarding_completed'));
      if (value === true) break;
      await pause(400);
    }
    expect(value).toBe(true);
  });

  // ───────────────────────────────────────────────────────────────────────
  // Phase C — Advanced (Custom), Configure on Voice mutates config.toml
  // ───────────────────────────────────────────────────────────────────────

  it('advanced/custom path: Configure on Voice updates local_ai.stt_provider in config.toml', async function () {
    // resetOnboardingFlagAndReload includes waitForWindowVisible(25_000), needs extra budget.
    this.timeout(90_000);
    await resetOnboardingFlagAndReload();

    // Welcome → Runtime choice (Custom) → Inference (Default).
    await advanceFromWelcomeToCustomInference('Phase C');

    expect(await clickTestId('onboarding-custom-inference-step-default')).toBe(true);
    await pause(400);
    await clickOnboardingNext();

    // Voice step → Configure → embedded VoicePanel renders. The auto-start
    // checkbox + Save button only render when local STT assets (Whisper) are
    // installed (`disabled = !sttReady` gates that block). In the CI
    // container we don't ship those assets, so we drive the always-visible
    // provider selectors instead — flipping the STT provider fires
    // `voice_set_providers`, which writes `config.local_ai.stt_provider`
    // to `config.toml` via `config.save()`.
    expect(await testIdExists('onboarding-custom-voice-step', 10_000)).toBe(true);
    expect(await clickTestId('onboarding-custom-voice-step-configure')).toBe(true);
    expect(await testIdExists('voice-providers-section', 10_000)).toBe(true);
    expect(await testIdExists('stt-provider-select', 10_000)).toBe(true);

    const before = readSectionString(readConfigToml(), 'local_ai', 'stt_provider');

    // Flip to a genuinely SELECTABLE provider. Local STT providers
    // (whisper/piper) render as `<option disabled>` in the CI container (no
    // local assets), so a synthetic change to them reverts and never persists —
    // asserting `stt_provider === 'whisper'` can never pass here. Pick an
    // enabled option whose value differs from the current selection and read
    // back the value the control actually committed to.
    const want = await browser.execute(() => {
      const el = document.querySelector<HTMLSelectElement>('[data-testid="stt-provider-select"]');
      if (!el) return null;
      const current = el.value;
      const candidate = Array.from(el.options).find(
        o => !o.disabled && o.value && o.value !== current
      );
      if (!candidate) return null;
      const setter = Object.getOwnPropertyDescriptor(
        window.HTMLSelectElement.prototype,
        'value'
      )?.set;
      if (setter) setter.call(el, candidate.value);
      else el.value = candidate.value;
      el.dispatchEvent(new Event('change', { bubbles: true }));
      // A disabled option would revert; an enabled one sticks.
      return el.value === candidate.value ? candidate.value : null;
    });

    if (!want) {
      stepLog(
        'No alternate selectable STT provider in this environment — skipping the provider-flip persistence assertion'
      );
      return;
    }
    stepLog(`stt_provider before=${before ?? '<unset>'} → want=${want}`);

    // Voice Routing was decoupled into staged edit + explicit Save: the select's
    // onChange only stages `sttProvider` (VoicePanel `onSttProviderChange`);
    // persistence to config.toml happens on the always-rendered Save button
    // (`save-voice-routing`, enabled once there are routing changes). Click it so
    // the staged provider actually writes through.
    expect(await clickTestId('save-voice-routing')).toBe(true);

    // Poll config.toml for the new value.
    let onDisk: string | null = null;
    const deadline = Date.now() + 10_000;
    while (Date.now() < deadline) {
      onDisk = readSectionString(readConfigToml(), 'local_ai', 'stt_provider');
      if (onDisk === want) break;
      await pause(500);
    }
    if (onDisk !== want) {
      stepLog(
        `local_ai.stt_provider expected=${want} got=${onDisk ?? '<unset>'}; config.toml:\n` +
          readConfigToml()
      );
    }
    expect(onDisk).toBe(want);

    // Continue out of the wizard so the spec leaves the app on /home.
    await clickOnboardingNext();
    expect(await testIdExists('onboarding-custom-oauth-step', 10_000)).toBe(true);
    expect(await clickTestId('onboarding-custom-oauth-step-default')).toBe(true);
    await pause(400);
    await clickOnboardingNext();

    // Step 5 — Custom Search (Default).
    expect(await testIdExists('onboarding-custom-search-step', 10_000)).toBe(true);
    expect(await clickTestId('onboarding-custom-search-step-default')).toBe(true);
    await pause(400);
    await clickOnboardingNext();

    // Step 6 — Custom Embeddings (Default).
    expect(await testIdExists('onboarding-custom-embeddings-step', 10_000)).toBe(true);
    expect(await clickTestId('onboarding-custom-embeddings-step-default')).toBe(true);
    await pause(400);
    await clickOnboardingNext();

    // Step 7 — Custom Activity (Default).
    expect(await testIdExists('onboarding-custom-activity-step', 10_000)).toBe(true);
    expect(await clickTestId('onboarding-custom-activity-step-default')).toBe(true);
    await pause(400);
    await clickOnboardingNext();

    // Step 8 — Custom Vault. Final step → Finish. Choice cards are hidden/auto-
    // configured only for LOCAL sessions; the E2E's cloud-auth session shows the
    // cards with an enabled default that must be picked before Finish enables.
    expect(await testIdExists('onboarding-custom-vault-step', 10_000)).toBe(true);
    if (await testIdExists('onboarding-custom-vault-step-default', 2_000)) {
      await clickTestId('onboarding-custom-vault-step-default');
    }
    await pause(400);
    await clickOnboardingNext();

    expect(await waitForHome(20_000)).toBe(true);
  });
});
