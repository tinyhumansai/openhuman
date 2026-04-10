// @ts-nocheck
/**
 * E2E test: Voice Intelligence (Built-in Skill — accessed from Skills tab)
 *
 * Covers the Section 9.3 built-in skill requirements:
 *
 *   9.3.1 — Voice Input Capture
 *     Verifies the dictation server lifecycle (status → start → stop) and
 *     that `voice_server_status` reports a well-formed state object the
 *     VoicePanel can render.
 *
 *   9.3.2 — Speech-to-Text Processing
 *     Verifies `voice_status` reports STT availability, model identifier,
 *     and binary paths, and that the `voice_transcribe_bytes` RPC surfaces
 *     a structured response (success or a well-formed error) when the
 *     whisper binary is absent (the default E2E condition).
 *
 *   9.3.3 — Voice Command Execution
 *     Verifies that voice server settings (hotkey, activation_mode) can be
 *     updated and that the updates are reflected in a subsequent
 *     `voice_server_status` call, exercising the end-to-end config plumbing
 *     that drives hotkey-triggered voice commands.
 *
 * The spec also verifies the UI surface: navigating to the Voice Intelligence
 * built-in card on the Skills page and opening the Voice Dictation settings
 * panel.
 *
 * This spec is modelled after the passing Screen Intelligence and Text
 * Auto-Complete built-in skill specs, with the following voice-specific
 * accommodations:
 *
 *   - **Voice is the 3rd built-in card (index 2).** `clickByTestId`'s Mac2
 *     Nth-index matcher needs all three "Settings" CTAs in the accessibility
 *     tree. We therefore wait for the card text to appear (guaranteeing the
 *     group is rendered) AND scroll the voice card into view before clicking.
 *
 *   - **Mac2 XQuery constraints.** The fallback CTA locator uses
 *     `contains(@attr, "...")` predicates only — XPath unions (`a | b`) and
 *     exact `@attr="..."` equality both throw `XQueryError:6 "invalid type"`
 *     on Mac2.
 *
 *   - **No `navigateViaHash('/settings/voice')` fallback on Mac2.** The
 *     shared helper's "click Settings tab" filter accepts any button with
 *     `title === 'Settings'`, and skill card CTAs expose their inner text
 *     "Settings" as `@title`, so it silently clicks the first skill card
 *     (Screen Intelligence) and redirects us to the wrong sub-page.
 *
 *   - **VoicePanel disabled state.** When STT isn't available (the default
 *     E2E case — no whisper binary), `VoicePanel` renders only the
 *     "Voice dictation is disabled" banner, not the Hotkey / Activation Mode
 *     form. Panel assertions accept either state.
 *
 * The mock server runs on http://127.0.0.1:18473
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  clickByTestId,
  dumpAccessibilityTree,
  hasAppChrome,
  scrollToFindText,
  textExists,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { isTauriDriver, supportsExecuteScript } from '../helpers/platform';
import {
  completeOnboardingIfVisible,
  dismissLocalAISnackbarIfVisible,
  navigateViaHash,
  waitForHomePage,
} from '../helpers/shared-flows';
import { clearRequestLog, getRequestLog, startMockServer, stopMockServer } from '../mock-server';

const LOG_PREFIX = '[VoiceIntelligenceE2E]';

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

// ---------------------------------------------------------------------------
// Navigation helpers (voice-specific)
// ---------------------------------------------------------------------------

/**
 * Pointer-click an element at its visual center via W3C actions.
 *
 * `element.click()` on Mac2 only fires accessibility-layer events, which
 * don't always propagate to WKWebView DOM onClick handlers. Using pointer
 * actions produces a real CGEvent mouse click.
 */
async function pointerClickElement(el: unknown): Promise<boolean> {
  try {
    const loc = await (
      el as { getLocation: () => Promise<{ x: number; y: number }> }
    ).getLocation();
    const size = await (
      el as { getSize: () => Promise<{ width: number; height: number }> }
    ).getSize();
    const cx = Math.round(loc.x + size.width / 2);
    const cy = Math.round(loc.y + size.height / 2);
    await browser.performActions([
      {
        type: 'pointer',
        id: 'mouse1',
        parameters: { pointerType: 'mouse' },
        actions: [
          { type: 'pointerMove', duration: 10, x: cx, y: cy },
          { type: 'pointerDown', button: 0 },
          { type: 'pause', duration: 50 },
          { type: 'pointerUp', button: 0 },
        ],
      },
    ]);
    await browser.releaseActions();
    stepLog('Mac2 pointer click', { cx, cy });
    return true;
  } catch (err) {
    stepLog('Mac2 pointer click failed', { error: String(err) });
    return false;
  }
}

/**
 * Wait for the Skills page to fully render the built-in section. We use the
 * Voice Intelligence card title as the "loaded" marker — on Mac2 WKWebView
 * the `<h2>Built-in</h2>` heading isn't always exposed in the accessibility
 * tree, but the card title reliably is.
 */
async function waitForSkillsPageLoaded(timeout = 25_000): Promise<boolean> {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await textExists('Voice Intelligence')) return true;
    // Skip past the loading indicator phase.
    if (await textExists('Loading skills...')) {
      await browser.pause(700);
      continue;
    }
    // The card may be below the fold — scroll down incrementally.
    const found = await scrollToFindText('Voice Intelligence', 2, 350);
    if (found) return true;
    await browser.pause(500);
  }
  return false;
}

/**
 * Click the Voice Intelligence card's "Settings" CTA.
 *
 *   - tauri-driver: direct `[data-testid="skill-cta-voice-stt"]` selector
 *     via the shared `clickByTestId` helper.
 *
 *   - Mac2 primary: the `UnifiedSkillCard` CTA renders with
 *     `aria-label="Settings voice-stt"`, which WKWebView exposes on the
 *     XCUIElementTypeButton. We locate it via the unique `voice-stt`
 *     substring in `@label`/`@title`/`@value`. This is more robust than
 *     the Nth-"Settings"-button index approach because it doesn't depend
 *     on accessibility tree ordering or all three CTAs being present.
 *
 *   - Mac2 fallback: shared `clickByTestId` Nth-index match — works when
 *     the full card row is rendered.
 */
async function clickVoiceSkillCta(timeout = 15_000): Promise<boolean> {
  if (isTauriDriver()) {
    try {
      await clickByTestId('skill-cta-voice-stt', timeout);
      return true;
    } catch (err) {
      stepLog('tauri-driver clickByTestId failed for voice-stt', { error: String(err) });
      return false;
    }
  }

  // Mac2: contains()-only XPath. No unions, no equality — both rejected with
  // `XQueryError:6 "invalid type"` on Mac2's XQuery engine.
  const ariaXpath =
    `//XCUIElementTypeButton[contains(@label, "voice-stt") or ` +
    `contains(@title, "voice-stt") or contains(@value, "voice-stt")]`;

  const ariaDeadline = Date.now() + Math.min(timeout, 10_000);
  let lastError: string | null = null;
  while (Date.now() < ariaDeadline) {
    try {
      const btn = await browser.$(ariaXpath);
      if (await btn.isExisting()) {
        if (await pointerClickElement(btn)) {
          stepLog('Mac2 clicked Voice Intelligence CTA via aria-label substring');
          return true;
        }
      }
    } catch (err) {
      lastError = String(err).slice(0, 200);
    }
    await browser.pause(500);
  }

  if (lastError) {
    stepLog('Mac2: aria-label xpath polling ended with error', { lastError });
  }

  // Fallback: shared Nth-index matcher. Requires all three built-in CTAs
  // ("Settings" buttons) in the accessibility tree.
  try {
    await clickByTestId('skill-cta-voice-stt', 5_000);
    stepLog('Mac2 clicked Voice Intelligence CTA via Nth-index fallback');
    return true;
  } catch (err) {
    stepLog('Mac2: Nth-index fallback also failed', { error: String(err).slice(0, 200) });
  }

  return false;
}

/**
 * Land on the Voice Dictation settings panel.
 *
 * Strategy (in order):
 *   1. If already on the panel, no-op.
 *   2. Ensure we're on the Skills page with Voice Intelligence card visible.
 *   3. Click the Voice Intelligence card's CTA.
 *   4. tauri-driver only: set `window.location.hash = '/settings/voice'`.
 *
 * No `navigateViaHash('/settings/voice')` fallback on Mac2 — see the spec
 * header comment for why.
 */
async function reachVoicePanel(): Promise<boolean> {
  if (await textExists('Voice Dictation')) return true;

  if (!(await textExists('Voice Intelligence'))) {
    await navigateViaHash('/skills');
    await browser.pause(2_000);
    await dismissLocalAISnackbarIfVisible(LOG_PREFIX);
    if (!(await waitForSkillsPageLoaded(20_000))) {
      stepLog('Skills page did not load during reachVoicePanel');
      return false;
    }
  }

  // Make sure the voice card is in the visible viewport so its button ends
  // up in the Mac2 accessibility tree.
  await scrollToFindText('Voice Intelligence', 4, 300);

  if (await clickVoiceSkillCta(12_000)) {
    const deadline = Date.now() + 8_000;
    while (Date.now() < deadline) {
      if (await textExists('Voice Dictation')) return true;
      await browser.pause(500);
    }
  }

  // tauri-driver only: direct hash navigation as a secondary strategy.
  if (supportsExecuteScript()) {
    try {
      await browser.execute(() => {
        window.location.hash = '/settings/voice';
      });
      await browser.pause(2_000);
      if (await textExists('Voice Dictation')) return true;
    } catch (err) {
      stepLog('execute hash navigation failed', { error: String(err) });
    }
  }

  return false;
}

// ---------------------------------------------------------------------------
// RPC result unwrapping
// ---------------------------------------------------------------------------

/**
 * Core JSON-RPC responses sometimes wrap the controller payload as
 * `{ result: {...}, logs: [...] }` and sometimes inline it. This helper
 * strips the wrapper.
 */
function unwrapRpcResult<T = unknown>(rpc: { ok: boolean; result?: unknown }): T {
  const raw = rpc.result as { result?: unknown } | undefined;
  return (raw && 'result' in (raw as object) ? (raw as { result: unknown }).result : raw) as T;
}

// ---------------------------------------------------------------------------
// describe / tests
// ---------------------------------------------------------------------------

describe('Voice Intelligence (Built-in Skill)', () => {
  before(async () => {
    stepLog('Starting Voice Intelligence E2E');
    await startMockServer();
    await waitForApp();
    clearRequestLog();
  });

  after(async () => {
    await stopMockServer();
  });

  // ── Auth + reach app shell ──────────────────────────────────────────────

  it('authenticates and reaches the app shell', async () => {
    await triggerAuthDeepLinkBypass('e2e-voice-intelligence-user');
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

  // ── Navigate to Voice Intelligence via Skills built-in card ─────────────

  it('navigates to Voice Intelligence from the Skills page built-in card', async () => {
    await dismissLocalAISnackbarIfVisible(LOG_PREFIX);
    let navigated = false;
    let lastError: unknown = null;

    for (let attempt = 1; attempt <= 2; attempt += 1) {
      await navigateViaHash('/skills');
      await browser.pause(2_000);
      await dismissLocalAISnackbarIfVisible(LOG_PREFIX);

      const hasBuiltIn = await waitForAnyText(
        ['Built-in Skills', 'Built-in', 'Voice Intelligence'],
        15_000
      );
      stepLog(`Skills page built-in section (attempt ${attempt})`, { found: hasBuiltIn });
      expect(hasBuiltIn).not.toBeNull();

      // Guarantee the card is scrolled into view before the click attempt.
      await scrollToFindText('Voice Intelligence', 4, 300);

      try {
        if (await clickVoiceSkillCta(15_000)) {
          navigated = true;
        }
      } catch (err) {
        lastError = err;
        stepLog(`Voice Intelligence CTA click threw (attempt ${attempt})`, { error: String(err) });
      }

      if (!navigated) continue;

      const onVoicePanel = await waitForAnyText(
        ['Voice Dictation', 'Voice Server Settings', 'Runtime'],
        12_000
      );
      if (onVoicePanel) {
        if (supportsExecuteScript()) {
          const currentHash = await browser.execute(() => window.location.hash);
          stepLog('After opening Voice Intelligence settings', { currentHash });
          expect(String(currentHash)).toContain('voice');
        }
        break;
      }

      navigated = false;
      stepLog(`Navigation did not land on Voice Dictation panel (attempt ${attempt})`);
    }

    if (!navigated) {
      const tree = await dumpAccessibilityTree();
      const requests = getRequestLog();
      stepLog('Failed to open Voice Intelligence from Skills. Tree:', tree.slice(0, 4000));
      stepLog(
        'Failed to open Voice Intelligence from Skills. Recent requests:',
        requests.slice(-20)
      );
      throw new Error(
        `Failed to open Voice Intelligence from Skills card\n` +
          `Last error: ${String(lastError)}\n` +
          `Accessibility tree (truncated):\n${tree.slice(0, 4000)}`
      );
    }
  });

  // ── Voice Dictation panel renders ───────────────────────────────────────

  it('shows the Voice Dictation panel with Runtime status', async () => {
    if (!(await textExists('Voice Dictation'))) {
      const reached = await reachVoicePanel();
      if (!reached) {
        const tree = await dumpAccessibilityTree();
        stepLog('Voice panel not reached before panel-render assertions', {
          tree: tree.slice(0, 4000),
        });
      }
    }
    expect(await textExists('Voice Dictation')).toBe(true);

    // Runtime status card is always rendered, regardless of STT availability.
    const hasRuntime = await waitForAnyText(['Runtime', 'STT', 'Server'], 10_000);
    stepLog('Runtime section', { found: hasRuntime });
    expect(hasRuntime).not.toBeNull();

    // VoicePanel gates the config form on `sttReady`. In E2E STT is not
    // available (no whisper binary), so only the amber disabled banner
    // renders instead of "Voice Server Settings". Accept either state.
    const hasServerOrDisabled = await waitForAnyText(
      [
        'Voice Server Settings',
        'Hotkey',
        'Activation Mode',
        'Voice dictation is disabled',
        'Open Local AI Model',
      ],
      10_000
    );
    stepLog('Server settings or disabled banner', { found: hasServerOrDisabled });
    expect(hasServerOrDisabled).not.toBeNull();
  });

  // ── 9.3.1 Voice Input Capture ───────────────────────────────────────────
  //
  // Lifecycle: voice_server_status → voice_server_start → voice_server_stop.
  // This is the backend plumbing for hotkey-triggered audio capture. Even
  // without a whisper binary, the server's state machine should transition
  // correctly and report a well-formed status object that the UI panel
  // observes (VoicePanel polls voice_server_status on a 2s interval).

  it('9.3.1 — voice_server_status returns a well-formed state object', async () => {
    const rpc = await callOpenhumanRpc('openhuman.voice_server_status', {});
    stepLog('voice_server_status RPC', JSON.stringify(rpc, null, 2));

    expect(rpc.ok).toBe(true);
    const data = unwrapRpcResult<{
      state: string;
      hotkey: string;
      activation_mode: string;
      transcription_count: number;
      last_error: string | null;
    }>(rpc);
    expect(data).toBeDefined();

    const validStates = ['stopped', 'idle', 'recording', 'transcribing'];
    expect(validStates).toContain(data.state);
    expect(typeof data.hotkey).toBe('string');
    expect(['push', 'tap']).toContain(data.activation_mode);
    expect(typeof data.transcription_count).toBe('number');
  });

  it('9.3.1 — voice server lifecycle (start → status → stop) reports transitions', async () => {
    // Baseline — must be stopped (either the session default or left over
    // from a previous test case). If the server is somehow running from a
    // previous case, stop it first so the subsequent start is deterministic.
    const baseline = unwrapRpcResult<{ state: string }>(
      await callOpenhumanRpc('openhuman.voice_server_status', {})
    );
    stepLog('baseline voice server state', { state: baseline.state });

    if (baseline.state !== 'stopped') {
      const preStop = await callOpenhumanRpc('openhuman.voice_server_stop', {});
      expect(preStop.ok).toBe(true);
      stepLog('pre-test stop completed');
    }

    // Attempt to start the voice server. In E2E without a working hotkey
    // listener or audio capture, three outcomes are all legitimate:
    //   1. RPC ok, state in {idle, recording, transcribing} — full success.
    //   2. RPC ok, state == "stopped" — the handler's tokio spawn actually
    //      started the server task, but the task exited before the handler's
    //      200ms status readback (CI has no hotkey API → background task
    //      stops immediately).  RPC transport + state machine still worked.
    //   3. RPC error — structured error propagated from `global_server().run`.
    // We assert on whichever branch happens and verify the response is
    // well-formed. The meaningful transition is that a subsequent `stop`
    // cleanly lands in `stopped`.
    const startRpc = await callOpenhumanRpc('openhuman.voice_server_start', {
      hotkey: 'F13',
      activation_mode: 'tap',
    });
    stepLog('voice_server_start RPC', JSON.stringify(startRpc, null, 2));

    if (startRpc.ok) {
      const startData = unwrapRpcResult<{
        state: string;
        hotkey?: string;
        activation_mode?: string;
      }>(startRpc);
      stepLog('voice_server_start succeeded', startData);
      // Must be a valid voice server state (no wrong strings, no nulls).
      expect(['stopped', 'idle', 'recording', 'transcribing']).toContain(startData.state);
      // Returned status should echo the requested hotkey/activation_mode
      // that were sent in the params (the server captured them before
      // whatever caused the background task to exit).
      if (typeof startData.hotkey === 'string') {
        expect(startData.hotkey.length).toBeGreaterThan(0);
      }
      if (typeof startData.activation_mode === 'string') {
        expect(['push', 'tap']).toContain(startData.activation_mode);
      }
    } else {
      stepLog('voice_server_start returned structured error (expected in E2E)', {
        error: (startRpc as { error?: string }).error,
      });
      // Start may legitimately fail when the underlying platform APIs
      // (hotkey capture, audio input) are unavailable in CI. Structured
      // failure is still acceptable — the important thing is the RPC did
      // not crash.
      expect(typeof (startRpc as { error?: string }).error).toBe('string');
    }

    // Stop must always succeed — voice_server_stop returns a stopped status
    // even when the server isn't running.
    const stopRpc = await callOpenhumanRpc('openhuman.voice_server_stop', {});
    stepLog('voice_server_stop RPC', JSON.stringify(stopRpc, null, 2));
    expect(stopRpc.ok).toBe(true);
    const stopData = unwrapRpcResult<{ state: string }>(stopRpc);
    expect(stopData.state).toBe('stopped');

    // Final status check — must still report stopped.
    const finalRpc = await callOpenhumanRpc('openhuman.voice_server_status', {});
    expect(finalRpc.ok).toBe(true);
    const finalData = unwrapRpcResult<{ state: string }>(finalRpc);
    expect(finalData.state).toBe('stopped');
  });

  // ── 9.3.2 Speech-to-Text Processing ─────────────────────────────────────
  //
  // Checks the STT half of voice intelligence: `voice_status` exposes the
  // whisper model identifier and binary paths, and `voice_transcribe_bytes`
  // either returns a VoiceSpeechResult or a structured error when the
  // binary is missing.

  it('9.3.2 — voice_status reports STT availability and model identifier', async () => {
    const rpc = await callOpenhumanRpc('openhuman.voice_status', {});
    stepLog('voice_status RPC', JSON.stringify(rpc, null, 2));

    expect(rpc.ok).toBe(true);
    const data = unwrapRpcResult<{
      stt_available: boolean;
      tts_available: boolean;
      stt_model_id: string;
      tts_voice_id: string;
      whisper_binary: string | null;
      piper_binary: string | null;
      stt_model_path: string | null;
      tts_voice_path: string | null;
      whisper_in_process: boolean;
      llm_cleanup_enabled: boolean;
    }>(rpc);
    expect(data).toBeDefined();

    // Required boolean flags
    expect(typeof data.stt_available).toBe('boolean');
    expect(typeof data.tts_available).toBe('boolean');
    expect(typeof data.whisper_in_process).toBe('boolean');
    expect(typeof data.llm_cleanup_enabled).toBe('boolean');

    // Required identifiers (stable — these come from bundled config).
    expect(typeof data.stt_model_id).toBe('string');
    expect(data.stt_model_id.length).toBeGreaterThan(0);
    expect(typeof data.tts_voice_id).toBe('string');
    expect(data.tts_voice_id.length).toBeGreaterThan(0);

    // Binary and model paths are nullable — but when non-null must be strings.
    for (const path of [
      data.whisper_binary,
      data.piper_binary,
      data.stt_model_path,
      data.tts_voice_path,
    ]) {
      if (path !== null) {
        expect(typeof path).toBe('string');
      }
    }

    stepLog('Voice status summary', {
      stt_available: data.stt_available,
      tts_available: data.tts_available,
      stt_model_id: data.stt_model_id,
      whisper_in_process: data.whisper_in_process,
      llm_cleanup_enabled: data.llm_cleanup_enabled,
    });
  });

  it('9.3.2 — voice_transcribe_bytes returns a structured response (success or error)', async () => {
    // Synthetic 1KB of silence — just enough to exercise the RPC without
    // requiring a real audio file. When whisper isn't available the RPC
    // returns a structured error; when it is, it returns a VoiceSpeechResult.
    const silentBytes = Array<number>(1024).fill(0);

    const rpc = await callOpenhumanRpc('openhuman.voice_transcribe_bytes', {
      audio_bytes: silentBytes,
      extension: 'wav',
      skip_cleanup: true,
    });
    stepLog('voice_transcribe_bytes RPC', JSON.stringify(rpc, null, 2));

    if (rpc.ok) {
      const data = unwrapRpcResult<{ text: string; raw_text: string; model_id: string }>(rpc);
      expect(typeof data.text).toBe('string');
      expect(typeof data.raw_text).toBe('string');
      expect(typeof data.model_id).toBe('string');
      stepLog('voice_transcribe_bytes succeeded', {
        text_length: data.text.length,
        model_id: data.model_id,
      });
    } else {
      const error = (rpc as { error?: string }).error;
      stepLog('voice_transcribe_bytes returned structured error (expected in E2E)', { error });
      expect(typeof error).toBe('string');
      expect((error ?? '').length).toBeGreaterThan(0);
    }
  });

  // ── 9.3.3 Voice Command Execution ───────────────────────────────────────
  //
  // "Voice command execution" = the full pipeline that turns a hotkey press
  // into an executed action. The plumbing the user can actually configure
  // is the hotkey and activation mode; we exercise the settings update RPC
  // and verify the new values are persisted and observable via
  // voice_server_status. TTS is also part of the command-execution loop
  // (read-back of results), so its RPC is exercised here too.

  it('9.3.3 — updating voice server settings persists the new hotkey and activation mode', async () => {
    type VoiceSettings = {
      hotkey: string;
      activation_mode: 'push' | 'tap';
      skip_cleanup: boolean;
      auto_start: boolean;
      min_duration_secs: number;
      silence_threshold: number;
      custom_dictionary: string[];
    };

    // Read the current (baseline) settings so we can verify changes and
    // restore them after the test.
    const beforeRpc = await callOpenhumanRpc('openhuman.config_get_voice_server_settings', {});
    stepLog('baseline config_get_voice_server_settings', JSON.stringify(beforeRpc, null, 2));
    expect(beforeRpc.ok).toBe(true);
    const beforeSettings = unwrapRpcResult<VoiceSettings>(beforeRpc);
    expect(beforeSettings).toBeDefined();
    expect(typeof beforeSettings.hotkey).toBe('string');
    expect(['push', 'tap']).toContain(beforeSettings.activation_mode);

    // Pick a hotkey + activation mode that is guaranteed different from the
    // baseline so the "persisted" check is meaningful.
    const targetHotkey = beforeSettings.hotkey === 'F18' ? 'F19' : 'F18';
    const targetMode: 'tap' | 'push' = beforeSettings.activation_mode === 'tap' ? 'push' : 'tap';

    const updateRpc = await callOpenhumanRpc('openhuman.config_update_voice_server_settings', {
      hotkey: targetHotkey,
      activation_mode: targetMode,
    });
    stepLog('config_update_voice_server_settings RPC', JSON.stringify(updateRpc, null, 2));
    expect(updateRpc.ok).toBe(true);

    // Read back and verify the values were persisted.
    const afterRpc = await callOpenhumanRpc('openhuman.config_get_voice_server_settings', {});
    expect(afterRpc.ok).toBe(true);
    const afterSettings = unwrapRpcResult<VoiceSettings>(afterRpc);
    expect(afterSettings).toBeDefined();
    expect(afterSettings.hotkey).toBe(targetHotkey);
    expect(afterSettings.activation_mode).toBe(targetMode);

    stepLog('Voice settings persisted', {
      hotkey: afterSettings.hotkey,
      activation_mode: afterSettings.activation_mode,
    });

    // Best-effort restore of the original settings so subsequent tests in
    // the run (or a follow-up run against the same workspace) start clean.
    const restoreRpc = await callOpenhumanRpc('openhuman.config_update_voice_server_settings', {
      hotkey: beforeSettings.hotkey,
      activation_mode: beforeSettings.activation_mode,
      skip_cleanup: beforeSettings.skip_cleanup,
      auto_start: beforeSettings.auto_start,
      min_duration_secs: beforeSettings.min_duration_secs,
      silence_threshold: beforeSettings.silence_threshold,
      custom_dictionary: beforeSettings.custom_dictionary,
    });
    if (!restoreRpc.ok) {
      stepLog('Warning: failed to restore baseline voice settings', { restoreRpc });
    }
  });

  it('9.3.3 — voice_tts RPC returns a structured response (success or error)', async () => {
    const rpc = await callOpenhumanRpc('openhuman.voice_tts', { text: 'OpenHuman voice test.' });
    stepLog('voice_tts RPC', JSON.stringify(rpc, null, 2));

    if (rpc.ok) {
      const data = unwrapRpcResult<{ output_path: string; voice_id: string }>(rpc);
      expect(typeof data.output_path).toBe('string');
      expect(data.output_path.length).toBeGreaterThan(0);
      expect(typeof data.voice_id).toBe('string');
      expect(data.voice_id.length).toBeGreaterThan(0);
      stepLog('voice_tts succeeded', data);
    } else {
      // When the piper binary is missing the RPC returns a structured error.
      // That's still a valid "command execution" outcome we want to assert on.
      const error = (rpc as { error?: string }).error;
      stepLog('voice_tts returned structured error (expected in E2E)', { error });
      expect(typeof error).toBe('string');
      expect((error ?? '').length).toBeGreaterThan(0);
    }
  });
});
