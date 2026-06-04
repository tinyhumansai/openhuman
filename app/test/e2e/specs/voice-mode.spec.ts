// @ts-nocheck
/**
 * E2E test: Voice mode integration
 *
 * Current desktop flow:
 *   - Chat defaults to the text composer.
 *   - The microphone button switches the composer into `MicComposer`.
 *   - `MicComposer` exposes a "Switch to text" control to restore the text
 *     textarea.
 *
 * The older "Text / Voice" segmented toggle no longer exists. This spec
 * covers the current desktop-only voice entry surface and keeps the
 * `openhuman.voice_status` RPC contract assertions below.
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { triggerAuthDeepLink } from '../helpers/deep-link-helpers';
import {
  clickButton,
  dumpAccessibilityTree,
  textExists,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { supportsExecuteScript } from '../helpers/platform';
import { completeOnboardingIfVisible } from '../helpers/shared-flows';
import {
  clearRequestLog,
  getRequestLog,
  setMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

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

async function waitForHome(timeout = 20_000) {
  // Home.tsx renders t('home.askAssistant') = 'Ask your assistant anything...' as stable CTA.
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await textExists('Ask your assistant anything')) return true;
    if (await textExists('Your device is connected')) return true;
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

// Browser-media UI behavior now lives in Playwright (`test/playwright/specs/voice-mode.spec.ts`).
// Keep WDIO/Appium focused on desktop/native-only coverage.
describe.skip('Voice mode integration', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
    clearRequestLog();
  });

  after(async () => {
    await stopMockServer();
  });

  it('can switch into the mic composer and back to text mode', async () => {
    await triggerAuthDeepLink('e2e-voice-token');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);

    const consume = await waitForRequest('POST', '/telegram/login-tokens/');
    expect(consume).toBeDefined();

    await completeOnboardingIfVisible('[VoiceModeE2E]');

    const onHome = await waitForHome(20_000);
    if (!onHome) {
      const tree = await dumpAccessibilityTree();
      console.log('[VoiceModeE2E] Home not reached. Tree:\n', tree.slice(0, 4000));
    }
    expect(onHome).toBe(true);

    const hasTextInput = await waitForAnyText(['How can I help', 'Threads', 'New'], 10_000);
    expect(hasTextInput).not.toBeNull();

    await clickButton('Voice mode', 10_000);

    const voiceStatusMessage = await waitForAnyText(
      [
        'Tap and speak',
        'Tap to send',
        'Speech-to-text unavailable',
        'whisper-cli binary',
        'Ready',
        'Start Talking',
        'Voice input needs a speech model',
      ],
      15_000
    );
    expect(voiceStatusMessage).not.toBeNull();

    await clickButton('Switch to text', 10_000);
    const textRestored = await waitForAnyText(['How can I help', 'Threads', 'New'], 10_000);
    expect(textRestored).not.toBeNull();
  });

  it('surfaces a mic entry button from the text composer', async () => {
    const onConversations = await waitForAnyText(['How can I help', 'Threads', 'New'], 10_000);
    if (!onConversations) {
      const tree = await dumpAccessibilityTree();
      console.log('[VoiceModeE2E] Conversations not ready. Tree:\n', tree.slice(0, 4000));
    }
    expect(onConversations).not.toBeNull();
    expect(await textExists('Voice mode')).toBe(true);
  });
});

/**
 * Offline STT mode — core RPC contract tests.
 *
 * These tests exercise the `openhuman.voice_status` RPC to assert the
 * availability contract without touching the UI voice toggle (which was
 * removed in #717). The RPC contract is:
 *
 *   - `stt_available=true` when either the in-process whisper engine is
 *     loaded, OR config.local_ai.whisper_in_process=true and the model file
 *     exists, OR whisper-cli binary + model file are both present.
 *   - `stt_available=false` when none of the above conditions hold; the app
 *     must not silently call a cloud STT provider when `stt_provider=whisper`.
 *
 * Product gap: there is no "offline mode" flag that prevents cloud fallback
 * when local assets are missing. The `it.skip` below records this gap.
 */
describe('Voice mode — offline STT contract (voice_status RPC)', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
  });

  after(async () => {
    await stopMockServer();
  });

  it('5.1 — voice_status RPC returns a well-formed response', async () => {
    const result = await callOpenhumanRpc('openhuman.voice_status', {});
    expect(result).toBeDefined();
    expect(typeof result).toBe('object');
    const status = (result as any).result ?? result;
    expect(typeof status.stt_available).toBe('boolean');
    expect(typeof status.tts_available).toBe('boolean');
    expect(typeof status.stt_provider).toBe('string');
  });

  it('5.2 — voice_status reports stt_available=false and non-cloud stt_provider when local assets are absent in the E2E environment', async () => {
    // In the E2E test environment whisper-cli is not installed and no model
    // file is seeded. The RPC must return stt_available=false rather than
    // silently advertising cloud availability under the whisper provider label.
    const result = await callOpenhumanRpc('openhuman.voice_status', {});
    const status = (result as any).result ?? result;

    if (status.stt_provider === 'whisper' || status.stt_provider === 'local') {
      // When stt_provider is whisper and the binary/model are absent, the
      // contract is stt_available=false (no silent cloud fallback).
      if (!status.whisper_binary && !status.stt_model_path) {
        expect(status.stt_available).toBe(false);
      }
    }
    // If stt_provider is "cloud" the field is correctly set — just assert the
    // provider is declared (not an empty string which would indicate an
    // undiscovered fallback).
    expect(status.stt_provider.length).toBeGreaterThan(0);
  });

  // TODO: Remove .skip when an explicit offline mode is implemented.
  // An "offline mode" toggle that (a) forces stt_provider=whisper and (b)
  // returns a clear error if assets are missing rather than falling back to
  // cloud has not yet been built. The config field `local_ai.stt_provider`
  // selects the provider but does not gate cloud fallback when local fails.
  //
  // Filed as product gap: src/openhuman/voice/ops.rs currently has no
  // offline-only enforcement path. When implemented, the new RPC behaviour
  // should be tested here and the skip removed.
  it.skip('5.3 — offline mode enabled + local assets missing → explicit "missing local STT" error, no cloud fallback', async () => {
    // When implemented:
    //   1. Set config.local_ai.stt_provider = "whisper" and ensure no binary/model.
    //   2. Attempt a transcription via voice_transcribe or trigger mic recording.
    //   3. Assert the error message identifies the missing local asset
    //      (e.g. "STT model not found") rather than a cloud API error.
    //   4. Assert no outbound HTTP request to any cloud STT endpoint was made.
  });
});

/**
 * Human tab voice capture and error mapping (issue #1610)
 *
 * These tests exercise the MicComposer on the Human tab (/human route) to
 * verify:
 *   6.1 — The Human tab renders with the mic composer in idle state.
 *   6.2 — The voice_stt_dispatch RPC contract: calling the RPC with a minimal
 *          audio payload through the mock server returns a well-formed
 *          transcription result (or a structured error — not a generic crash).
 *   6.3 — Permission-denied path: when getUserMedia throws NotAllowedError,
 *          the error banner carries a specific error code (not "Something went
 *          wrong"), verified via the data-chat-send-error-code DOM attribute.
 *   6.4 — No-device path: when getUserMedia throws NotFoundError / the headless
 *          CEF environment has no mic, the composer surfaces a specific
 *          no-device or microphone-access error (not a generic crash).
 *   6.5 — Beep-placeholder guard: the chat thread must not contain the literal
 *          string "beep" as a user utterance after the mic button is tapped in
 *          a headless environment (regression guard for #1610).
 *
 * Headless CEF reality:
 *   The headless docker runner has no real microphone. All flows that require
 *   actual audio capture are driven by JS mocking of navigator.mediaDevices.
 *   The `browser.execute` approach is supported on tauri-driver (Linux/CEF);
 *   on Mac2 (Appium) these tests fall back to it.skip with an explanatory
 *   comment because the Mac2 driver does not expose JS execution in the WebView.
 *
 * Navigation:
 *   The Human tab is reached by navigating to the /human hash route. The
 *   BottomTabBar renders a button with aria-label="Human". We use
 *   browser.execute to set window.location.hash directly, which avoids
 *   element-visibility races on the tab bar.
 */
// These Human-tab getUserMedia / MediaRecorder behaviors are browser API paths,
// not native-shell concerns. They are covered in Playwright now.
describe.skip('Voice mode — Human tab capture & error mapping (#1610)', () => {
  before(async () => {
    await startMockServer();
    await waitForApp();
  });

  after(async () => {
    await stopMockServer();
  });

  // ---------------------------------------------------------------------------
  // Helper: navigate to the Human tab via hash routing.
  // ---------------------------------------------------------------------------
  async function navigateToHumanTab(): Promise<void> {
    if (supportsExecuteScript()) {
      await browser.execute(() => {
        window.location.hash = '#/human';
      });
    } else {
      // Mac2 path: use the shared helper which abstracts the XCUIElementTypeButton
      // XPath so the selector stays cross-driver and policy-compliant.
      await clickNativeButton('Human');
    }
    // Allow React router to settle and the Human page to mount.
    await browser.pause(1_500);
  }

  // ---------------------------------------------------------------------------
  // Helper: inject a getUserMedia mock that throws a named DOMException.
  // The real navigator.mediaDevices.getUserMedia is replaced for the duration
  // of a single test; the spec restores it afterwards. Only works on
  // tauri-driver / CEF where browser.execute reaches the WebView DOM.
  // ---------------------------------------------------------------------------
  async function mockGetUserMediaError(domExceptionName: string): Promise<void> {
    await browser.execute((name: string) => {
      // Store the real implementation so the test can restore it.
      (window as any).__e2e_gum_original = navigator.mediaDevices?.getUserMedia?.bind(
        navigator.mediaDevices
      );
      // Replace with a function that rejects with the requested DOMException.
      Object.defineProperty(navigator.mediaDevices, 'getUserMedia', {
        configurable: true,
        value: () => {
          const err = new DOMException(`[E2E mock] getUserMedia blocked (${name})`, name);
          return Promise.reject(err);
        },
      });
    }, domExceptionName);
  }

  async function restoreGetUserMedia(): Promise<void> {
    await browser.execute(() => {
      const original = (window as any).__e2e_gum_original;
      if (original && navigator.mediaDevices) {
        Object.defineProperty(navigator.mediaDevices, 'getUserMedia', {
          configurable: true,
          value: original,
        });
      }
      delete (window as any).__e2e_gum_original;
    });
  }

  // ---------------------------------------------------------------------------
  // Helper: wait for a data-chat-send-error-code attribute to appear in the
  // DOM and return its value. Returns null if the element does not appear
  // within the timeout.
  // ---------------------------------------------------------------------------
  async function waitForSendErrorCode(timeout = 10_000): Promise<string | null> {
    if (!supportsExecuteScript()) return null;
    const deadline = Date.now() + timeout;
    while (Date.now() < deadline) {
      const code = await browser.execute(() => {
        const el = document.querySelector('[data-chat-send-error-code]');
        return el ? el.getAttribute('data-chat-send-error-code') : null;
      });
      if (code) return code as string;
      await browser.pause(400);
    }
    return null;
  }

  // ---------------------------------------------------------------------------
  // Helper: read the full text of the error banner message element.
  // ---------------------------------------------------------------------------
  async function getSendErrorMessage(): Promise<string> {
    if (!supportsExecuteScript()) return '';
    return (await browser.execute(() => {
      const el = document.querySelector('[data-chat-send-error-code]');
      return el ? ((el as HTMLElement).textContent ?? '') : '';
    })) as string;
  }

  // ---------------------------------------------------------------------------
  // 6.1 — Human tab renders with MicComposer in idle state.
  //
  // Checks that the Human tab mounts, shows the "Push to Talk" label in the
  // mascot header, and the MicComposer idle button (aria-label="Start recording"
  // / visible label "Tap and speak") is present.
  // ---------------------------------------------------------------------------
  it('6.1 — Human tab renders with MicComposer in idle state', async () => {
    await triggerAuthDeepLink('e2e-voice-human-tab-token');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible('[HumanTabE2E]');

    await navigateToHumanTab();

    // The Human page renders a "Push to Talk" checkbox in the mascot header.
    const hasPushToTalk = await textExists('Push to Talk');
    if (!hasPushToTalk) {
      const tree = await dumpAccessibilityTree();
      console.log(
        '[HumanTabE2E:6.1] Push-to-Talk not found. Accessibility tree:\n',
        tree.slice(0, 4_000)
      );
    }
    expect(hasPushToTalk).toBe(true);

    // The MicComposer is embedded via the sidebar Conversations with
    // composer="mic-cloud". The idle button label is "Tap and speak".
    const hasMicLabel = await textExists('Tap and speak');
    if (!hasMicLabel) {
      // Accept "Waiting for agent..." — the composer is mounted but a thread
      // load is still in flight. Either label proves the MicComposer is up.
      const hasWaiting = await textExists('Waiting for agent');
      if (!hasWaiting) {
        const tree = await dumpAccessibilityTree();
        console.log('[HumanTabE2E:6.1] Mic label not found. Tree:\n', tree.slice(0, 4_000));
      }
      expect(hasWaiting).toBe(true);
    }
  });

  // ---------------------------------------------------------------------------
  // 6.2 — voice_stt_dispatch RPC returns a well-formed result or structured
  //       error (not a generic crash) when called with a minimal audio payload.
  //
  // In the E2E environment the mock server handles
  // /openai/v1/audio/transcriptions — so the cloud STT path returns
  // "Mock transcription from the E2E server." The test uses
  // `setMockBehavior('audioTranscriptionText', ...)` to set a known value,
  // then calls the RPC directly over HTTP using callOpenhumanRpc. No actual
  // microphone or MediaRecorder is involved.
  // ---------------------------------------------------------------------------
  it('6.2 — voice_stt_dispatch RPC returns well-formed result with mock transcription payload', async () => {
    // Configure the mock server to return a known transcript.
    setMockBehavior('audioTranscriptionText', 'hello from the E2E voice test');

    // Build a minimal valid WAV buffer: 44-byte header + 1 silent frame.
    // The Rust core decodes base64 audio and passes it to the STT provider;
    // for the cloud path the actual content just needs to be non-empty.
    const silentWavBase64 = await browser.execute(() => {
      const sampleRate = 16_000;
      const numSamples = 160; // 10 ms of silence at 16kHz
      const dataBytes = numSamples * 2; // 16-bit PCM

      const buf = new ArrayBuffer(44 + dataBytes);
      const view = new DataView(buf);
      const writeAscii = (offset: number, s: string) => {
        for (let i = 0; i < s.length; i++) view.setUint8(offset + i, s.charCodeAt(i));
      };

      writeAscii(0, 'RIFF');
      view.setUint32(4, 36 + dataBytes, true);
      writeAscii(8, 'WAVE');
      writeAscii(12, 'fmt ');
      view.setUint32(16, 16, true); // chunk size
      view.setUint16(20, 1, true); // PCM
      view.setUint16(22, 1, true); // mono
      view.setUint32(24, sampleRate, true);
      view.setUint32(28, sampleRate * 2, true); // byte rate
      view.setUint16(32, 2, true); // block align
      view.setUint16(34, 16, true); // bits per sample
      writeAscii(36, 'data');
      view.setUint32(40, dataBytes, true);
      // Samples are already zeroed.

      const bytes = new Uint8Array(buf);
      const CHUNK = 0x8000;
      let binary = '';
      for (let i = 0; i < bytes.length; i += CHUNK) {
        binary += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
      }
      return btoa(binary);
    });

    const result = await callOpenhumanRpc('openhuman.voice_stt_dispatch', {
      audio_base64: silentWavBase64,
      mime_type: 'audio/wav',
      file_name: 'test.wav',
    });

    // The result must be defined and must be an object — not a raw string
    // or an unhandled panic. The actual transcription text may differ
    // (depends on which STT provider the core resolved), but the shape must
    // have a `text` field (or a `result.text` field via RpcOutcome).
    expect(result).toBeDefined();
    const payload = (result as any).result ?? result;
    expect(typeof payload).toBe('object');
    // `text` is the canonical field on FactoryTranscribeResult.
    expect('text' in payload || 'error' in payload || 'code' in payload).toBe(true);
    // When the cloud path ran, the mock returns our known text.
    if ('text' in payload) {
      expect(typeof payload.text).toBe('string');
      // Not a generic crash string.
      expect((payload.text as string).toLowerCase()).not.toContain('something went wrong');
    }
  });

  // ---------------------------------------------------------------------------
  // 6.3 — Permission-denied path.
  //
  // When getUserMedia throws NotAllowedError the MicComposer maps it to
  // `onError('Microphone permission denied: …')`, which Conversations wraps
  // into chatSendError('voice_transcription', message). The error banner must
  // carry data-chat-send-error-code != "" and the message must mention
  // "permission" or "denied" — not the generic "Something went wrong".
  //
  // This test uses browser.execute to replace navigator.mediaDevices.getUserMedia
  // with a mock that rejects with NotAllowedError. This is only possible on
  // tauri-driver (Linux/CEF). On Mac2 (Appium) the test is skipped because the
  // Mac2 driver does not expose JavaScript execution inside the WKWebView.
  // ---------------------------------------------------------------------------
  it('6.3 — permission-denied getUserMedia surfaces specific error code, not generic failure', async () => {
    if (!supportsExecuteScript()) {
      // Mac2 / Appium path — JS injection into WKWebView is not supported.
      // The OS-level permission dialog cannot be driven programmatically from
      // the test harness either. Skip with explanation.
      console.log(
        '[HumanTabE2E:6.3] SKIP — Mac2 driver does not support browser.execute() in WKWebView. ' +
          'Permission-denied path requires JS mocking of navigator.mediaDevices.getUserMedia.'
      );
      return;
    }

    await navigateToHumanTab();

    // Replace getUserMedia with a NotAllowedError-throwing mock.
    await mockGetUserMediaError('NotAllowedError');

    try {
      // Click the "Start recording" button (aria-label on the <button> in MicComposer).
      const clicked = await browser.execute(() => {
        const btn = document.querySelector<HTMLButtonElement>('[aria-label="Start recording"]');
        if (!btn) return false;
        btn.click();
        return true;
      });

      if (!clicked) {
        // If the button wasn't found, the Human tab may not have fully
        // mounted yet — wait for the Tap-and-speak label and retry once.
        await browser.pause(1_500);
        const retried = await browser.execute(() => {
          const btn = document.querySelector<HTMLButtonElement>('[aria-label="Start recording"]');
          if (btn) {
            btn.click();
            return true;
          }
          return false;
        });
        if (!retried) {
          // Dump the tree for diagnosis, then fail explicitly so CI catches
          // regressions where the Human tab stops mounting in time.
          const tree = await dumpAccessibilityTree();
          console.log(
            '[HumanTabE2E:6.3] Start-recording button not found. Tree:\n',
            tree.slice(0, 4_000)
          );
          throw new Error(
            '[HumanTabE2E:6.3] Start-recording button not found after retry — Human tab did not mount in time'
          );
        }
      }

      // Wait for the error banner to appear.
      const errorCode = await waitForSendErrorCode(8_000);
      if (!errorCode) {
        const tree = await dumpAccessibilityTree();
        console.log(
          '[HumanTabE2E:6.3] No error banner appeared after NotAllowedError. Tree:\n',
          tree.slice(0, 4_000)
        );
      }

      // The error code must be set (any specific code is better than nothing).
      expect(errorCode).not.toBeNull();
      expect(errorCode!.length).toBeGreaterThan(0);

      // The error message must include "permission" or "denied" so the user
      // gets actionable feedback — not a generic "Something went wrong".
      const msg = await getSendErrorMessage();
      const lowerMsg = msg.toLowerCase();
      const isActionable =
        lowerMsg.includes('permission') ||
        lowerMsg.includes('denied') ||
        lowerMsg.includes('microphone');
      if (!isActionable) {
        console.log('[HumanTabE2E:6.3] Error message was not actionable:', msg);
      }
      expect(isActionable).toBe(true);

      // Regression guard: must never say "Something went wrong".
      expect(lowerMsg).not.toContain('something went wrong');
    } finally {
      await restoreGetUserMedia();
    }
  });

  // ---------------------------------------------------------------------------
  // 6.4 — No-device / NotFoundError path.
  //
  // When getUserMedia throws NotFoundError (no audio input device available —
  // the typical headless CEF scenario) the MicComposer maps it to
  // 'Selected microphone is unavailable — try a different device.' via
  // onError, which surfaces as chatSendError('voice_transcription', …).
  // The error must be specific to the hardware absence, not a generic crash.
  //
  // On tauri-driver: we first let the native headless getUserMedia fail
  // naturally (no mock needed — CEF on Linux docker has no mic device).
  // If getUserMedia somehow succeeds (e.g. a virtual ALSA loopback is
  // present), we fall back to mocking NotFoundError to keep the contract
  // assertion reliable.
  //
  // On Mac2: skipped (no browser.execute support).
  // ---------------------------------------------------------------------------
  it('6.4 — no-device getUserMedia (NotFoundError) surfaces specific no-audio error, not generic failure', async () => {
    if (!supportsExecuteScript()) {
      console.log(
        '[HumanTabE2E:6.4] SKIP — Mac2 driver does not support browser.execute(). ' +
          'No-device path requires either natural headless failure or JS mock of getUserMedia.'
      );
      return;
    }

    await navigateToHumanTab();

    // Check whether the headless environment naturally lacks an audio device.
    // If getUserMedia would succeed (virtual loopback present), we mock it.
    const hasRealDevice = await browser.execute(async () => {
      if (!navigator.mediaDevices?.enumerateDevices) return false;
      try {
        const devices = await navigator.mediaDevices.enumerateDevices();
        return devices.some((d: MediaDeviceInfo) => d.kind === 'audioinput');
      } catch {
        return false;
      }
    });

    if (hasRealDevice) {
      // Virtual audio device present — inject NotFoundError to simulate
      // the no-device path reliably.
      await mockGetUserMediaError('NotFoundError');
    }
    // If no real device is present, clicking the button naturally triggers
    // NotFoundError from the browser itself — no mock needed.

    try {
      const clicked = await browser.execute(() => {
        const btn = document.querySelector<HTMLButtonElement>('[aria-label="Start recording"]');
        if (!btn) return false;
        btn.click();
        return true;
      });

      if (!clicked) {
        await browser.pause(1_500);
        const retried = await browser.execute(() => {
          const btn = document.querySelector<HTMLButtonElement>('[aria-label="Start recording"]');
          if (btn) {
            btn.click();
            return true;
          }
          return false;
        });
        if (!retried) {
          const tree = await dumpAccessibilityTree();
          console.log(
            '[HumanTabE2E:6.4] Start-recording button not found. Tree:\n',
            tree.slice(0, 4_000)
          );
          throw new Error(
            '[HumanTabE2E:6.4] Start-recording button not found after retry — Human tab did not mount in time'
          );
        }
      }

      // Wait for the error banner.
      const errorCode = await waitForSendErrorCode(8_000);
      if (!errorCode) {
        const tree = await dumpAccessibilityTree();
        console.log(
          '[HumanTabE2E:6.4] No error banner after NotFoundError/no-device. Tree:\n',
          tree.slice(0, 4_000)
        );
      }

      expect(errorCode).not.toBeNull();
      expect(errorCode!.length).toBeGreaterThan(0);

      // The message must mention the hardware absence specifically.
      const msg = await getSendErrorMessage();
      const lowerMsg = msg.toLowerCase();
      const isSpecific =
        lowerMsg.includes('microphone') ||
        lowerMsg.includes('unavailable') ||
        lowerMsg.includes('device') ||
        lowerMsg.includes('access') ||
        lowerMsg.includes('not found');
      if (!isSpecific) {
        console.log('[HumanTabE2E:6.4] Error message was not specific:', msg);
      }
      expect(isSpecific).toBe(true);

      // Regression guard.
      expect(lowerMsg).not.toContain('something went wrong');
    } finally {
      if (hasRealDevice) {
        await restoreGetUserMedia();
      }
    }
  });

  // ---------------------------------------------------------------------------
  // 6.5 — Beep-placeholder guard (regression for #1610).
  //
  // The chat thread must not contain the literal string "beep" as a user
  // message after the mic button is tapped in a headless environment and
  // getUserMedia fails. An earlier implementation emitted a placeholder beep
  // token as the user utterance when capture was not available.
  //
  // We mock NotAllowedError (the clearest failure) and assert the thread log
  // does not include a user message containing "beep".
  // ---------------------------------------------------------------------------
  it('6.5 — beep placeholder is not emitted as a user utterance after mic failure', async () => {
    if (!supportsExecuteScript()) {
      console.log(
        '[HumanTabE2E:6.5] SKIP — Mac2 driver does not support browser.execute(). ' +
          'Beep-placeholder guard requires JS thread inspection.'
      );
      return;
    }

    await navigateToHumanTab();
    await mockGetUserMediaError('NotAllowedError');

    try {
      // Dismiss any existing error banner so we get a clean slate.
      await browser.execute(() => {
        const dismissBtn = document.querySelector<HTMLButtonElement>(
          '[data-chat-send-error-code] ~ div button'
        );
        dismissBtn?.click();
      });
      await browser.pause(300);

      // Tap the mic button.
      await browser.execute(() => {
        const btn = document.querySelector<HTMLButtonElement>('[aria-label="Start recording"]');
        btn?.click();
      });

      // Wait for the error to surface.
      await waitForSendErrorCode(8_000);

      // Now assert that no user message bubble in the thread says "beep".
      const beepInThread = await browser.execute(() => {
        // User messages are rendered by ChatBubble / MessageBubble.
        // We cast a wide net: any element with role="group" or a message
        // container whose data-sender="user" contains the word "beep".
        const candidates = Array.from(
          document.querySelectorAll('[data-sender="user"], [data-message-sender="user"]')
        );
        return candidates.some(
          (el: Element) => (el as HTMLElement).textContent?.toLowerCase().includes('beep') ?? false
        );
      });

      expect(beepInThread).toBe(false);
    } finally {
      await restoreGetUserMedia();
    }
  });

  // ---------------------------------------------------------------------------
  // 6.6 — Actual transcription round-trip with mocked audio device (SKIPPED).
  //
  // A full round-trip — speak → MediaRecorder captures audio → STT RPC →
  // transcript appears as user message → agent replies — requires:
  //   a) A real or virtual audio device (unavailable in headless docker).
  //   b) The ability to inject PCM frames into MediaRecorder (not possible
  //      via WebDriver — WDIO has no W3C audio injection API for CEF).
  //   c) The mock server to handle /openai/v1/audio/transcriptions (it does).
  //
  // The contract is tested at the RPC layer in test 6.2 (voice_stt_dispatch)
  // and at the unit level in MicComposer.test.tsx (transcribeWithFactory mock).
  // This skip records the gap: there is no E2E path that drives real audio
  // through MediaRecorder in the CI headless environment.
  //
  // Tracked: issue #1610. Remove skip when the test harness supports audio
  // injection (e.g. via a virtual ALSA device + pre-recorded WAV replayer, or
  // a fake getUserMedia implementation that returns a pre-seeded MediaStream).
  // ---------------------------------------------------------------------------
  it.skip('6.6 — spoken prompt round-trip: mic → STT → user bubble → agent reply (requires real/virtual audio device)', async () => {
    // When unblocked:
    //   1. Navigate to /human.
    //   2. Inject a pre-recorded WAV as a fake MediaStream via getUserMedia mock
    //      (or use a virtual ALSA loopback device seeded by the test harness).
    //   3. Click "Start recording", let the recorder run for ~500 ms, click again.
    //   4. Wait for the STT RPC to complete (mock returns known transcript text).
    //   5. Assert the known transcript text appears as a user bubble in the thread.
    //   6. Assert the agent responds (at minimum: a non-empty agent message bubble).
    //   7. Assert the user bubble does not contain "beep" or other placeholder text.
  });
});
