// @ts-nocheck
/** Full mocked recording -> transcription -> assistant speech -> reload flow. */
import { waitForApp } from '../helpers/app-helpers';
import {
  chatMounted,
  clickByTitle,
  getSelectedThreadId,
  waitForAssistantReplyContaining,
} from '../helpers/chat-harness';
import {
  clickTestId,
  clickTestIdAtIndex,
  countTestId,
  textExists,
  waitForDecodedAudioAtIndex,
  waitForTestId,
  waitForTestIdAtIndex,
} from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import {
  clearRequestLog,
  getRequestLog,
  setMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

const USER_ID = 'e2e-chat-voice-note';
const TRANSCRIPT = 'Please remind me about the dentist';
const REPLY = 'I will remind you about the dentist.';

function silentWavBase64() {
  const sampleRate = 8000;
  const samples = new Uint8Array(sampleRate * 5);
  const bytes = new Uint8Array(44 + samples.length);
  const view = new DataView(bytes.buffer);
  const tag = (offset, value) =>
    [...value].forEach((char, index) => (bytes[offset + index] = char.charCodeAt(0)));
  tag(0, 'RIFF');
  view.setUint32(4, 36 + samples.length, true);
  tag(8, 'WAVE');
  tag(12, 'fmt ');
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate, true);
  view.setUint16(32, 1, true);
  view.setUint16(34, 8, true);
  tag(36, 'data');
  view.setUint32(40, samples.length, true);
  bytes.set(samples, 44);
  return Buffer.from(bytes).toString('base64');
}

const AUDIO_B64 = silentWavBase64();

describe('Chat voice notes', () => {
  before(async function () {
    this.timeout(90_000);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
    setMockBehavior('audioTranscriptionText', TRANSCRIPT);
    setMockBehavior('audioSpeechBase64', AUDIO_B64);
    setMockBehavior('audioSpeechMime', 'audio/wav');
    setMockBehavior(
      'audioSpeechVisemes',
      JSON.stringify([
        { viseme: 'aa', start_ms: 0, end_ms: 180 },
        { viseme: 'PP', start_ms: 180, end_ms: 420 },
      ])
    );
    setMockBehavior('llmStreamScript', JSON.stringify([{ text: REPLY }, { finish: 'stop' }]));
    clearRequestLog();
  });

  after(async () => {
    setMockBehavior('audioTranscriptionText', '');
    setMockBehavior('audioSpeechBase64', '');
    setMockBehavior('audioSpeechMime', '');
    setMockBehavior('audioSpeechVisemes', '');
    setMockBehavior('llmStreamScript', '');
    await stopMockServer();
  });

  it('records, sends the transcription, receives voiced reply, reloads, and replays both notes', async function () {
    this.timeout(180_000);
    await navigateViaHash('/chat');
    await browser.waitUntil(async () => await chatMounted(), { timeout: 15_000 });
    expect(await clickByTitle('New thread', 8_000)).toBe(true);
    const threadId = await browser.waitUntil(async () => await getSelectedThreadId(), {
      timeout: 8_000,
    });

    // The browser recorder emits a valid short WAV so both stored user and synthesized
    // assistant audio can be decoded by the real player without a microphone/device.
    await browser.execute(wav => {
      const raw = Uint8Array.from(atob(wav), char => char.charCodeAt(0));
      Object.defineProperty(navigator, 'mediaDevices', {
        configurable: true,
        value: { getUserMedia: async () => ({ getTracks: () => [{ stop() {} }] }) },
      });
      window.MediaRecorder = class {
        static isTypeSupported() {
          return true;
        }
        constructor() {
          this.state = 'inactive';
          this.mimeType = 'audio/wav';
          this.listeners = {};
        }
        addEventListener(type, fn) {
          this.listeners[type] = fn;
        }
        start() {
          this.state = 'recording';
        }
        stop() {
          this.state = 'inactive';
          this.listeners.dataavailable?.({ data: new Blob([raw], { type: 'audio/wav' }) });
          this.listeners.stop?.();
        }
      };
    }, AUDIO_B64);

    await clickTestId('voice-note-record');
    await waitForTestId('voice-note-recording', 5_000);
    await clickTestId('voice-note-stop');
    await waitForTestId('voice-note-preview', 5_000);
    await clickTestId('voice-note-send');
    await browser.waitUntil(async () => await textExists(TRANSCRIPT), { timeout: 20_000 });
    await waitForAssistantReplyContaining(REPLY, 60_000);
    const audioRequests = getRequestLog().filter(request =>
      ['/openai/v1/audio/transcriptions', '/openai/v1/audio/speech'].includes(request.url)
    );
    expect(audioRequests.map(request => request.url)).toEqual(
      expect.arrayContaining(['/openai/v1/audio/transcriptions', '/openai/v1/audio/speech'])
    );
    await browser.waitUntil(async () => (await countTestId('voice-note-player')) === 2, {
      timeout: 30_000,
    });

    await browser.refresh();
    await waitForApp();
    await navigateViaHash('/chat');
    await browser.waitUntil(async () => await chatMounted(), { timeout: 20_000 });
    await browser.execute(
      tid =>
        window.__OPENHUMAN_STORE__?.dispatch({ type: 'thread/setSelectedThread', payload: tid }),
      threadId
    );
    await browser.waitUntil(async () => (await countTestId('voice-note-player')) === 2, {
      timeout: 30_000,
    });
    expect(await textExists(REPLY)).toBe(true);
    await clickTestId('voice-note-transcript-toggle');
    await browser.waitUntil(async () => await textExists(TRANSCRIPT), { timeout: 5_000 });

    await browser.waitUntil(async () => (await countTestId('voice-note-toggle')) === 2, {
      timeout: 5_000,
    });
    await browser.execute(() => {
      const NativeAudio = window.Audio;
      window.__E2E_AUDIO_ELEMENTS = [];
      window.Audio = class extends NativeAudio {
        constructor(source) {
          super(source);
          window.__E2E_AUDIO_ELEMENTS.push(this);
        }
      };
    });
    await clickTestIdAtIndex('voice-note-toggle', 0);
    await waitForDecodedAudioAtIndex(0);
    await browser.waitUntil(
      async () =>
        (await (await waitForTestId('voice-note-toggle')).getAttribute('aria-label'))
          ?.toLowerCase()
          .includes('pause'),
      { timeout: 5_000 }
    );
    await clickTestIdAtIndex('voice-note-toggle', 0);
    await clickTestIdAtIndex('voice-note-toggle', 1);
    await waitForDecodedAudioAtIndex(1);
    await browser.waitUntil(
      async () =>
        (await (await waitForTestIdAtIndex('voice-note-toggle', 1)).getAttribute('aria-label'))
          ?.toLowerCase()
          .includes('pause'),
      { timeout: 5_000 }
    );
    await clickTestIdAtIndex('voice-note-toggle', 1);
  });
});
