import { expect, test } from '@playwright/test';

import {
  type CaptureFailure,
  captureState,
  createNewThread,
  installFakeCapture,
  mockDictationRpc,
  openChat,
  setCaptureFailure,
  waitForSocketConnected,
} from '../../../playwright/helpers/dictation';
import { webElements } from '../../helpers/element-helpers';

test.describe('Chat composer inline dictation', () => {
  test('keeps the draft editable, inserts one final transcript, and waits for Send', async ({
    page,
  }) => {
    await installFakeCapture(page);
    const { sttCalls, sentMessages } = await mockDictationRpc(page);
    const input = await openChat(page, 'pw-composer-dictation-edit');
    await input.replaceComposerText('Typed first');

    await webElements(page).button('Dictate').click();
    await expect.poll(() => webElements(page).button('Finish dictation').isVisible()).toBe(true);
    await expect.poll(() => input.isEditable()).toBe(true);
    await input.click();
    await input.press('End');
    await input.type(' and edited while speaking');
    await expect.poll(() => input.composerText()).toBe('Typed first and edited while speaking');
    expect(sentMessages).toEqual([]);

    await webElements(page).button('Finish dictation').click();
    const draft = 'Typed first and edited while speaking dictated final words';
    await expect.poll(() => input.composerText()).toBe(draft);
    expect(sttCalls).toHaveLength(1);
    expect(sttCalls[0].params.audio_base64).toBeTruthy();
    expect(sttCalls[0].params.mime_type).toBe('audio/webm');
    expect(sttCalls[0].params).not.toHaveProperty('provider');
    expect(sentMessages).toEqual([]);
    await expect.poll(async () => (await captureState(page)).tracksStopped).toBe(1);

    await input.click();
    await input.press('End');
    await input.type(' — reviewed');
    await waitForSocketConnected(page);
    await expect
      .poll(() => webElements(page).byTestId('send-message-button').isEnabled())
      .toBe(true);
    await webElements(page).byTestId('send-message-button').click();
    await expect.poll(() => sentMessages).toEqual([`${draft} — reviewed`]);
  });

  for (const sttResult of ['success', 'wav-only'] as const) {
    const expectedAttempts = sttResult === 'wav-only' ? 2 : 1;
    for (const cancellation of ['Discard dictation', 'Escape'] as const) {
      test(`${cancellation} preserves typed text and ignores a pending ${sttResult} STT result`, async ({
        page,
      }) => {
        await installFakeCapture(page);
        const rpc = await mockDictationRpc(page, { holdTranscript: true, sttResult });
        const input = await openChat(
          page,
          `pw-composer-dictation-${cancellation.split(' ')[0]}-${sttResult}`
        );
        await input.replaceComposerText('Keep this draft');
        await webElements(page).button('Dictate').click();
        await expect
          .poll(() => webElements(page).button('Finish dictation').isVisible())
          .toBe(true);
        await webElements(page).button('Finish dictation').click();
        await expect.poll(() => rpc.sttCalls.length).toBe(expectedAttempts);

        if (cancellation === 'Escape') {
          await input.click();
          await input.press('Escape');
        } else {
          await webElements(page).button(cancellation).click();
        }
        await expect.poll(() => webElements(page).button('Dictate').isVisible()).toBe(true);
        await input.click();
        await input.press('End');
        await input.type(' and keep editing');
        await rpc.releaseTranscript('late transcript that must be ignored', expectedAttempts);
        expect((await captureState(page)).sttSettled).toBe(expectedAttempts);
        await input.type(' after cancellation');

        await expect
          .poll(() => input.composerText())
          .toBe('Keep this draft and keep editing after cancellation');
        await expect.poll(() => webElements(page).button('Finish dictation').count()).toBe(0);
        expect(rpc.sentMessages).toEqual([]);
        await expect.poll(async () => (await captureState(page)).tracksStopped).toBe(1);
      });
    }

    test(`switching threads ignores the previous composer’s pending ${sttResult} transcript`, async ({
      page,
    }) => {
      await installFakeCapture(page);
      const rpc = await mockDictationRpc(page, { holdTranscript: true, sttResult });
      await openChat(page, `pw-composer-dictation-thread-switch-${sttResult}`);
      const input = webElements(page).byTestId('chat-message-input');
      await input.replaceComposerText('First thread draft');
      await webElements(page).button('Dictate').click();
      await expect.poll(() => webElements(page).button('Finish dictation').isVisible()).toBe(true);
      await webElements(page).button('Finish dictation').click();
      await expect.poll(() => rpc.sttCalls.length).toBe(expectedAttempts);

      await createNewThread(page);
      await expect.poll(() => webElements(page).button('Dictate').isVisible()).toBe(true);
      await input.replaceComposerText('Second thread draft');
      await rpc.releaseTranscript('late words from the first thread', expectedAttempts);
      expect((await captureState(page)).sttSettled).toBe(expectedAttempts);
      await input.type(' checked');

      await expect.poll(() => input.composerText()).toBe('Second thread draft checked');
      expect(rpc.sentMessages).toEqual([]);
      await expect.poll(async () => (await captureState(page)).tracksStopped).toBe(1);
    });
  }

  for (const capability of ['missing', 'unavailable'] as const) {
    test(`hides Dictate when the core voice capability is ${capability}`, async ({ page }) => {
      await installFakeCapture(page);
      const rpc = await mockDictationRpc(page, { capability });
      const input = await openChat(page, `pw-composer-dictation-${capability}`);
      await expect.poll(() => rpc.voiceStatusCalls.length).toBeGreaterThan(0);
      await input.replaceComposerText('Typing remains available');

      await expect.poll(() => webElements(page).button('Dictate').count()).toBe(0);
      await expect.poll(() => input.isEditable()).toBe(true);
      expect((await captureState(page)).permissionRequests).toBe(0);
    });
  }

  test('hides Dictate when browser recording is unsupported', async ({ page }) => {
    await installFakeCapture(page, { unsupported: true });
    await mockDictationRpc(page);
    const input = await openChat(page, 'pw-composer-dictation-unsupported');

    await expect.poll(() => webElements(page).button('Dictate').count()).toBe(0);
    await expect.poll(() => input.isEditable()).toBe(true);
    expect((await captureState(page)).permissionRequests).toBe(0);
  });

  test('explains unavailable speech and recovers on focus without switching threads', async ({
    page,
  }) => {
    await installFakeCapture(page);
    const rpc = await mockDictationRpc(page, { capability: 'unavailable' });
    const input = await openChat(page, 'pw-composer-dictation-recovery');
    await input.replaceComposerText('Keep my draft');
    const error = webElements(page).alert('Dictation is unavailable');
    await expect.poll(() => error.isVisible()).toBe(true);
    await expect.poll(() => webElements(page).button('Dictate').count()).toBe(0);
    const threadUrl = page.url();

    rpc.setCapability('available');
    await page.evaluate(() => window.dispatchEvent(new Event('focus')));
    await expect.poll(() => webElements(page).button('Dictate').isVisible()).toBe(true);
    await expect.poll(() => error.count()).toBe(0);
    expect(page.url()).toBe(threadUrl);
    expect(await input.composerText()).toBe('Keep my draft');
    expect((await captureState(page)).permissionRequests).toBe(0);
  });

  test('permission denial shows an actionable error and keeps the draft', async ({ page }) => {
    await installFakeCapture(page, { permissionDenied: true });
    const { sttCalls, sentMessages } = await mockDictationRpc(page);
    const input = await openChat(page, 'pw-composer-dictation-permission');
    await input.replaceComposerText('My draft stays');
    await webElements(page).button('Dictate').click();

    const error = webElements(page).alert('Microphone permission denied');
    await expect.poll(() => error.isVisible()).toBe(true);
    await expect.poll(() => error.text()).toMatch(/permission|denied|microphone/i);
    await expect.poll(() => webElements(page).button('Dictate').isEnabled()).toBe(true);
    await expect.poll(() => input.isEditable()).toBe(true);
    expect(await input.composerText()).toBe('My draft stays');
    expect(sttCalls).toEqual([]);
    expect(sentMessages).toEqual([]);
    expect((await captureState(page)).recordingsStarted).toBe(0);
  });
});

type FailureCase = {
  capture?: CaptureFailure;
  stt?: 'error' | 'empty';
  alert: string;
  finish?: boolean;
  timeout?: boolean;
  releasedTracks: number;
  sttCalls: number;
};

const recoverableErrors = {
  'microphone-unavailable': {
    capture: 'AbortError',
    alert: 'Microphone is not available',
    releasedTracks: 0,
    sttCalls: 0,
  },
  'permission-denied': {
    capture: 'NotAllowedError',
    alert: 'Microphone permission denied',
    releasedTracks: 0,
    sttCalls: 0,
  },
  'device-unavailable': {
    capture: 'NotFoundError',
    alert: 'Selected microphone is unavailable',
    releasedTracks: 0,
    sttCalls: 0,
  },
  'device-in-use': {
    capture: 'NotReadableError',
    alert: 'Microphone is in use',
    releasedTracks: 0,
    sttCalls: 0,
  },
  'recorder-failed': {
    capture: 'recorder',
    alert: 'Failed to start recorder',
    releasedTracks: 1,
    sttCalls: 0,
  },
  'no-audio': {
    capture: 'empty',
    alert: 'No audio captured',
    finish: true,
    releasedTracks: 1,
    sttCalls: 0,
  },
  'no-speech': {
    stt: 'empty',
    alert: 'No speech detected',
    finish: true,
    releasedTracks: 1,
    sttCalls: 1,
  },
  'transcription-failed': {
    stt: 'error',
    alert: 'Transcription failed',
    finish: true,
    releasedTracks: 1,
    sttCalls: 2,
  },
  'timed-out': { alert: 'Dictation timed out', timeout: true, releasedTracks: 1, sttCalls: 0 },
} satisfies Record<string, FailureCase>;

test.describe('Chat composer dictation errors and fallback', () => {
  for (const [code, failure] of Object.entries<FailureCase>(recoverableErrors)) {
    test(`${code} releases capture, preserves the draft, and allows a successful retry`, async ({
      page,
    }) => {
      await installFakeCapture(page, { failure: failure.capture });
      const rpc = await mockDictationRpc(page, { sttResult: failure.stt });
      const input = await openChat(page, `pw-dictation-error-${code}`);
      await input.replaceComposerText('Keep this draft');
      if (failure.timeout) await page.clock.install();
      await webElements(page).button('Dictate').click();
      if (failure.finish || failure.timeout) {
        await expect
          .poll(() => webElements(page).button('Finish dictation').isVisible())
          .toBe(true);
      }
      if (failure.finish) await webElements(page).button('Finish dictation').click();
      if (failure.timeout) await page.clock.fastForward(60_001);

      const alert = webElements(page).alert(failure.alert);
      await expect.poll(() => alert.isVisible()).toBe(true);
      expect(await alert.text()).not.toContain('Private');
      expect(await input.composerText()).toBe('Keep this draft');
      expect(await input.isEditable()).toBe(true);
      await expect.poll(() => webElements(page).button('Dictate').isEnabled()).toBe(true);
      expect(await webElements(page).button('Discard dictation').count()).toBe(0);
      expect((await captureState(page)).tracksStopped).toBe(failure.releasedTracks);
      expect(rpc.sttCalls).toHaveLength(failure.sttCalls);
      expect(rpc.sentMessages).toEqual([]);

      await setCaptureFailure(page);
      rpc.setSttResult('success');
      await input.click();
      await input.press('End');
      await input.type(' and edited');
      await webElements(page).button('Dictate').click();
      await expect.poll(() => webElements(page).button('Finish dictation').isVisible()).toBe(true);
      await expect.poll(() => alert.count()).toBe(0);
      await webElements(page).button('Finish dictation').click();
      await expect
        .poll(() => input.composerText())
        .toBe('Keep this draft and edited dictated final words');
      await expect.poll(() => webElements(page).button('Dictate').isEnabled()).toBe(true);
      expect((await captureState(page)).tracksStopped).toBe(failure.releasedTracks + 1);
      expect(rpc.sttCalls).toHaveLength(failure.sttCalls + 1);
      expect(rpc.sentMessages).toEqual([]);
    });
  }

  test('a failed voice-status RPC explains availability and recovers in the same composer', async ({
    page,
  }) => {
    await installFakeCapture(page);
    const rpc = await mockDictationRpc(page, { capability: 'error' });
    const input = await openChat(page, 'pw-dictation-status-error');
    await input.replaceComposerText('Keep this draft');
    const alert = webElements(page).alert('Could not check dictation availability');
    await expect.poll(() => alert.isVisible()).toBe(true);
    expect(await alert.text()).not.toContain('Private');
    expect(await webElements(page).button('Dictate').count()).toBe(0);
    expect(await input.isEditable()).toBe(true);
    expect((await captureState(page)).permissionRequests).toBe(0);
    expect(rpc.sttCalls).toEqual([]);
    const threadUrl = page.url();

    rpc.setCapability('available');
    await page.evaluate(() => window.dispatchEvent(new Event('focus')));
    await expect.poll(() => webElements(page).button('Dictate').isEnabled()).toBe(true);
    await expect.poll(() => alert.count()).toBe(0);
    expect(page.url()).toBe(threadUrl);
    expect(await input.composerText()).toBe('Keep this draft');
    await webElements(page).button('Dictate').click();
    await webElements(page).button('Finish dictation').click();
    await expect.poll(() => input.composerText()).toBe('Keep this draft dictated final words');
    expect((await captureState(page)).tracksStopped).toBe(1);
    expect(rpc.sentMessages).toEqual([]);
  });

  test('a missing STT method withdraws dictation, preserves the draft, and rechecks for the next thread', async ({
    page,
  }) => {
    await installFakeCapture(page);
    const rpc = await mockDictationRpc(page, { sttResult: 'missing' });
    const input = await openChat(page, 'pw-dictation-missing-stt');
    await input.replaceComposerText('Keep this draft');
    await webElements(page).button('Dictate').click();
    await webElements(page).button('Finish dictation').click();
    const alert = webElements(page).alert('Voice transcription is not included');
    await expect.poll(() => alert.isVisible()).toBe(true);
    expect(await input.composerText()).toBe('Keep this draft');
    expect(await input.isEditable()).toBe(true);
    expect(await webElements(page).button('Dictate').count()).toBe(0);
    expect(await webElements(page).button('Discard dictation').count()).toBe(0);
    expect((await captureState(page)).tracksStopped).toBe(1);
    expect(rpc.sttCalls).toHaveLength(1);
    expect(rpc.sentMessages).toEqual([]);

    rpc.setSttResult('success');
    await createNewThread(page);
    await input.replaceComposerText('Next thread');
    await webElements(page).button('Dictate').click();
    await expect.poll(() => alert.count()).toBe(0);
    await webElements(page).button('Finish dictation').click();
    await expect.poll(() => input.composerText()).toBe('Next thread dictated final words');
    expect((await captureState(page)).tracksStopped).toBe(2);
    expect(rpc.sentMessages).toEqual([]);
  });

  test('retries rejected native audio as real PCM WAV and appends the final transcript once', async ({
    page,
  }) => {
    await installFakeCapture(page);
    const rpc = await mockDictationRpc(page, { sttResult: 'wav-only', holdTranscript: true });
    const input = await openChat(page, 'pw-dictation-wav-fallback');
    await input.replaceComposerText('Typed first');
    await webElements(page).button('Dictate').click();
    await webElements(page).button('Finish dictation').click();
    await expect.poll(() => rpc.sttCalls.length).toBe(2);
    expect(await input.composerText()).toBe('Typed first');
    await rpc.releaseTranscript('dictated final words', 2);
    expect((await captureState(page)).sttSettled).toBe(2);
    await expect.poll(() => input.composerText()).toBe('Typed first dictated final words');
    await expect.poll(() => webElements(page).button('Dictate').isEnabled()).toBe(true);
    expect(rpc.sttCalls.map(call => call.params.mime_type)).toEqual(['audio/webm', 'audio/wav']);
    const native = Buffer.from(String(rpc.sttCalls[0].params.audio_base64), 'base64');
    expect(native.subarray(0, 4).toString('hex')).toBe('1a45dfa3'); // EBML/WebM header
    const wav = Buffer.from(String(rpc.sttCalls[1].params.audio_base64), 'base64');
    expect(wav.toString('ascii', 0, 4)).toBe('RIFF');
    expect(wav.toString('ascii', 8, 12)).toBe('WAVE');
    expect(wav.readUInt16LE(20)).toBe(1); // PCM
    expect(wav.readUInt16LE(22)).toBe(1); // mono
    expect(wav.readUInt32LE(24)).toBe(16_000);
    expect(wav.readUInt16LE(34)).toBe(16);
    expect(wav.readUInt32LE(40)).toBe(wav.length - 44);
    expect(wav.subarray(44).some(byte => byte !== 0)).toBe(true);
    expect(rpc.sttCalls[1].params.file_name).toBe('audio.wav');
    expect((await captureState(page)).tracksStopped).toBe(1);
    expect(rpc.sentMessages).toEqual([]);
    // Editing after completion proves the composer is idle and has one result.
    await input.click();
    await input.press('End');
    await input.type(' reviewed');
    expect(await input.composerText()).toBe('Typed first dictated final words reviewed');
    expect(rpc.sttCalls).toHaveLength(2);
  });
});
