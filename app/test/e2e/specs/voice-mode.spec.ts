// @ts-nocheck
/**
 * E2E test: Voice Intelligence (Built-in Skill — accessed from Skills tab)
 *
 * Covers:
 *   9.3.1 — Navigate to Skills page and verify Voice Intelligence built-in card
 *   9.3.2 — Click Voice Intelligence card → opens /settings/voice panel
 *   9.3.3 — Voice Dictation settings panel renders with key sections (Runtime, Settings)
 *   9.3.4 — voice_status RPC returns STT/TTS availability info
 *   9.3.5 — voice_server_status RPC returns server state
 *   9.3.6 — Voice configuration options render (Hotkey, Activation Mode, Writing Style)
 *
 * The mock server runs on http://127.0.0.1:18473
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  clickText,
  dumpAccessibilityTree,
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

const LOG_PREFIX = '[VoiceModeE2E]';

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
    await triggerAuthDeepLinkBypass('e2e-voice-token');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible(LOG_PREFIX);

    const home = await waitForHomePage(20_000);
    if (!home) {
      const tree = await dumpAccessibilityTree();
      stepLog('Home not reached', { tree: tree.slice(0, 4000) });
    }
    expect(home).not.toBeNull();
  });

  // ── 9.3.1 Skills page shows Voice Intelligence built-in card ────────────

  it('shows the Voice Intelligence card in the Skills tab', async () => {
    await dismissLocalAISnackbarIfVisible(LOG_PREFIX);
    await navigateViaHash('/skills');
    await browser.pause(2_000);

    const hasBuiltIn = await waitForAnyText(['Built-in Skills'], 10_000);
    stepLog('Built-in Skills section', { found: hasBuiltIn });
    expect(hasBuiltIn).not.toBeNull();

    const hasVoiceCard = await waitForAnyText(['Voice Intelligence'], 10_000);
    stepLog('Voice Intelligence card', { found: hasVoiceCard });
    expect(hasVoiceCard).not.toBeNull();

    const hasDescription = await textExists('Use the microphone');
    stepLog('Voice card description', { hasDescription });
    expect(hasDescription).toBe(true);
  });

  // ── 9.3.2 Click card → opens Voice settings panel ──────────────────────

  it('navigates to Voice settings panel from Skills tab', async () => {
    await clickText('Voice Intelligence', 10_000);
    await browser.pause(2_000);

    if (supportsExecuteScript()) {
      const currentHash = await browser.execute(() => window.location.hash);
      stepLog('After clicking Voice Intelligence card', { currentHash });
      expect(currentHash).toContain('voice');
    }

    const hasPanel = await waitForAnyText(
      ['Voice Dictation', 'Voice Server Settings', 'Runtime'],
      15_000
    );
    if (!hasPanel) {
      const tree = await dumpAccessibilityTree();
      stepLog('Voice panel missing expected headings', { tree: tree.slice(0, 4000) });
    }
    stepLog('Voice settings panel', { found: hasPanel });
    expect(hasPanel).not.toBeNull();
  });

  // ── 9.3.3 Voice Dictation panel renders key sections ────────────────────

  it('shows key sections in the Voice Dictation panel', async () => {
    const alreadyOnPage = await textExists('Voice Dictation');
    if (!alreadyOnPage) {
      await navigateViaHash('/settings/voice');
      await browser.pause(2_000);
    }

    const hasRuntime = await waitForAnyText(['Runtime', 'STT', 'Server'], 10_000);
    stepLog('Runtime section', { found: hasRuntime });
    expect(hasRuntime).not.toBeNull();

    // Check for voice server settings heading
    const hasServerSettings = await waitForAnyText(
      ['Voice Server Settings', 'Hotkey', 'Activation Mode'],
      10_000
    );
    stepLog('Server settings section', { found: hasServerSettings });
    expect(hasServerSettings).not.toBeNull();
  });

  // ── 9.3.4 voice_status RPC ─────────────────────────────────────────────

  it('voice_status RPC returns STT/TTS availability info', async () => {
    const result = await callOpenhumanRpc('openhuman.voice_status', {});
    stepLog('voice_status RPC raw', JSON.stringify(result, null, 2));

    expect(result.ok).toBe(true);

    const raw = result.result;
    const data = raw?.result ?? raw;
    expect(data).toBeDefined();

    expect(typeof data.stt_available).toBe('boolean');
    expect(typeof data.tts_available).toBe('boolean');
    expect(typeof data.stt_model_id).toBe('string');
    expect(typeof data.tts_voice_id).toBe('string');

    stepLog('Voice availability', {
      stt_available: data.stt_available,
      tts_available: data.tts_available,
      whisper_binary: data.whisper_binary,
      piper_binary: data.piper_binary,
      whisper_in_process: data.whisper_in_process,
    });
  });

  // ── 9.3.5 voice_server_status RPC ──────────────────────────────────────

  it('voice_server_status RPC returns server state', async () => {
    const result = await callOpenhumanRpc('openhuman.voice_server_status', {});
    stepLog('voice_server_status RPC raw', JSON.stringify(result, null, 2));

    expect(result.ok).toBe(true);

    const raw = result.result;
    const data = raw?.result ?? raw;
    expect(data).toBeDefined();

    const validStates = ['stopped', 'idle', 'recording', 'transcribing'];
    expect(validStates).toContain(data.state);
    expect(typeof data.hotkey).toBe('string');
    expect(typeof data.activation_mode).toBe('string');

    stepLog('Voice server state', {
      state: data.state,
      hotkey: data.hotkey,
      activation_mode: data.activation_mode,
      transcription_count: data.transcription_count,
    });
  });

  // ── 9.3.6 Voice configuration options ──────────────────────────────────

  it('shows voice configuration options (Hotkey, Activation Mode, Writing Style)', async () => {
    const alreadyOnPage = await textExists('Voice Dictation');
    if (!alreadyOnPage) {
      await navigateViaHash('/settings/voice');
      await browser.pause(2_000);
    }

    // These labels should be visible without deep scrolling
    const configLabels = [
      'Hotkey',
      'Activation Mode',
      'Writing Style',
      'Save Voice Settings',
      'Start Voice Server',
      'Stop Voice Server',
    ];

    const foundLabels: string[] = [];
    for (const label of configLabels) {
      if (await textExists(label)) foundLabels.push(label);
    }

    stepLog('Voice config labels found', { foundLabels });
    // At least 2 should be visible
    expect(foundLabels.length).toBeGreaterThanOrEqual(2);
  });
});
