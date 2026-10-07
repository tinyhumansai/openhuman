import { expect, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

// The shared E2E entry point re-exports these from web-elements.ts for browser tests.
import { webElements, type WebTestElement } from '../../e2e/helpers/element-helpers';
import { bootAuthenticatedPage, dismissWalkthroughIfPresent } from './core-rpc';

interface CaptureState {
  permissionRequests: number;
  recordingsStarted: number;
  recordingsStopped: number;
  tracksStopped: number;
  sttSettled: number;
}

interface RpcCall {
  method: string;
  params: Record<string, unknown>;
}

type VoiceCapability = 'available' | 'unavailable' | 'missing' | 'error';
export type CaptureFailure =
  | 'NotAllowedError'
  | 'NotFoundError'
  | 'NotReadableError'
  | 'AbortError'
  | 'recorder'
  | 'empty';
type SttResult = 'success' | 'error' | 'missing' | 'empty' | 'wav-only';
type CaptureWindow = Window & {
  __dictationCapture: CaptureState;
  __dictationFailure?: CaptureFailure;
};

// Real synthetic WebM/Opus data lets the production WAV encoder decode audio.
// Capture is synthetic; the AudioContext and encoder implementation remain real.
const audioBase64 = readFileSync(
  fileURLToPath(new URL('../fixtures/dictation-tone.webm', import.meta.url))
).toString('base64');

/** Install before bootstrap: the suite never opens an actual microphone. */
export async function installFakeCapture(
  page: Page,
  options: { permissionDenied?: boolean; unsupported?: boolean; failure?: CaptureFailure } = {}
): Promise<void> {
  await page.route(
    url =>
      ['http:', 'https:'].includes(url.protocol) &&
      !['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname),
    route => route.abort()
  );
  await page.addInitScript(
    ({ permissionDenied, unsupported, failure, audioBase64 }) => {
      const state: CaptureState = {
        permissionRequests: 0,
        recordingsStarted: 0,
        recordingsStopped: 0,
        tracksStopped: 0,
        sttSettled: 0,
      };
      const captureWindow = window as unknown as CaptureWindow;
      captureWindow.__dictationCapture = state;
      captureWindow.__dictationFailure = permissionDenied ? 'NotAllowedError' : failure;
      const audioBytes = Uint8Array.from(atob(audioBase64), char => char.charCodeAt(0));

      // Observe STT settlement so cancellation assertions wait for the late
      // response (or its abort), rather than passing before it reaches the app.
      const originalFetch = window.fetch.bind(window);
      window.fetch = (input, init) => {
        let isStt = false;
        if (typeof init?.body === 'string') {
          try {
            isStt = JSON.parse(init.body).method === 'openhuman.voice_stt_dispatch';
          } catch {}
        }
        const response = originalFetch(input, init);
        if (!isStt) return response;
        return response.then(
          reply => {
            const originalJson = reply.json.bind(reply);
            reply.json = () => originalJson().finally(() => (state.sttSettled += 1));
            return reply;
          },
          error => {
            state.sttSettled += 1;
            throw error;
          }
        );
      };

      const mediaDevices = new EventTarget();
      Object.assign(mediaDevices, {
        enumerateDevices: async () => [],
        getUserMedia: async () => {
          state.permissionRequests += 1;
          const failure = captureWindow.__dictationFailure;
          if (failure?.endsWith('Error')) {
            throw new DOMException('Private capture failure detail', failure);
          }
          const track = Object.assign(new EventTarget(), {
            kind: 'audio',
            enabled: true,
            readyState: 'live',
            stop() {
              if (this.readyState === 'ended') return;
              this.readyState = 'ended';
              state.tracksStopped += 1;
            },
          });
          return { active: true, getTracks: () => [track], getAudioTracks: () => [track] };
        },
      });
      Object.defineProperty(navigator, 'mediaDevices', { configurable: true, value: mediaDevices });

      class FakeMediaRecorder extends EventTarget {
        static isTypeSupported(mime: string): boolean {
          return mime.startsWith('audio/webm');
        }

        state = 'inactive';
        mimeType: string;
        ondataavailable: ((event: BlobEvent) => void) | null = null;
        onstop: ((event: Event) => void) | null = null;
        onerror: ((event: Event) => void) | null = null;

        constructor(_stream: MediaStream, options?: MediaRecorderOptions) {
          super();
          this.mimeType = options?.mimeType ?? 'audio/webm';
        }

        start(): void {
          if (captureWindow.__dictationFailure === 'recorder') {
            throw new Error('Private recorder failure detail');
          }
          this.state = 'recording';
          state.recordingsStarted += 1;
        }

        stop(): void {
          if (this.state === 'inactive') return;
          this.state = 'inactive';
          state.recordingsStopped += 1;
          queueMicrotask(() => {
            const data = new BlobEvent('dataavailable', {
              data: new Blob(captureWindow.__dictationFailure === 'empty' ? [] : [audioBytes], {
                type: this.mimeType,
              }),
            });
            this.dispatchEvent(data);
            this.ondataavailable?.(data);
            const stopped = new Event('stop');
            this.dispatchEvent(stopped);
            this.onstop?.(stopped);
          });
        }
      }

      Object.defineProperty(window, 'MediaRecorder', {
        configurable: true,
        value: unsupported ? undefined : FakeMediaRecorder,
      });
    },
    { ...options, audioBase64 }
  );
}

/** Repair the fake device without remounting the composer or replacing its draft. */
export async function setCaptureFailure(page: Page, failure?: CaptureFailure): Promise<void> {
  await page.evaluate(failure => {
    (window as unknown as CaptureWindow).__dictationFailure = failure;
  }, failure);
}

/** Read capture cleanup and RPC settlement counters from the browser fixture. */
export async function captureState(page: Page): Promise<CaptureState> {
  return page.evaluate(
    () => (window as unknown as { __dictationCapture: CaptureState }).__dictationCapture
  );
}

/** Control speech replies while keeping authentication and other RPCs on the local core. */
export async function mockDictationRpc(
  page: Page,
  options: { capability?: VoiceCapability; holdTranscript?: boolean; sttResult?: SttResult } = {}
): Promise<{
  voiceStatusCalls: RpcCall[];
  sttCalls: RpcCall[];
  sentMessages: string[];
  setCapability: (capability: VoiceCapability) => void;
  setSttResult: (result: SttResult) => void;
  releaseTranscript: (text: string, expectedAttempts: number) => Promise<void>;
}> {
  let capability = options.capability ?? 'available';
  let sttResult = options.sttResult ?? 'success';
  const voiceStatusCalls: RpcCall[] = [];
  const sttCalls: RpcCall[] = [];
  const sentMessages: string[] = [];
  let resolveTranscript: ((text: string) => void) | undefined;
  const transcript = options.holdTranscript
    ? new Promise<string>(resolve => (resolveTranscript = resolve))
    : Promise.resolve('dictated final words');

  await page.route('**/rpc', async (route, request) => {
    const body = JSON.parse(request.postData() || '{}');
    const fulfill = (result: unknown) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ jsonrpc: '2.0', id: body.id, result }),
      });
    const reject = (code: number, message: string) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ jsonrpc: '2.0', id: body.id, error: { code, message } }),
      });

    if (body.method === 'openhuman.voice_status') {
      voiceStatusCalls.push({ method: body.method, params: body.params });
      if (capability === 'missing' || capability === 'error') {
        await reject(
          capability === 'missing' ? -32601 : -32603,
          capability === 'missing'
            ? 'unknown method: openhuman.voice_status'
            : 'Private status detail'
        );
      } else {
        await fulfill({
          stt_available: capability !== 'unavailable',
          tts_available: true,
          stt_engine: 'hosted',
          stt_error: null,
        });
      }
      return;
    }
    if (body.method === 'openhuman.voice_stt_dispatch') {
      sttCalls.push({ method: body.method, params: body.params });
      if (sttResult === 'missing') {
        await reject(-32601, 'unknown method: openhuman.voice_stt_dispatch');
        return;
      }
      if (
        sttResult === 'error' ||
        (sttResult === 'wav-only' && body.params.mime_type !== 'audio/wav')
      ) {
        await reject(-32603, 'Private transcription failure detail');
        return;
      }
      const text = await transcript;
      // A canceled fetch may have closed its route while the result was held.
      await fulfill({
        text: sttResult === 'empty' ? '  ' : text,
        provider: 'configured-stt',
      }).catch(() => {});
      return;
    }
    if (body.method === 'openhuman.channel_web_chat') {
      sentMessages.push(String(body.params.message));
      await fulfill({ accepted: true });
      return;
    }
    await route.fallback();
  });

  return {
    voiceStatusCalls,
    sttCalls,
    sentMessages,
    setCapability: next => {
      capability = next;
    },
    setSttResult: next => {
      sttResult = next;
    },
    releaseTranscript: async (text, expectedAttempts) => {
      // Wait for the held request to exist before releasing it. A rejected native
      // request can already be settled while its WAV retry is still in flight.
      await expect.poll(() => sttCalls.length).toBe(expectedAttempts);
      resolveTranscript?.(text);
      await expect.poll(async () => (await captureState(page)).sttSettled).toBe(expectedAttempts);
    },
  };
}

/** Bootstrap a mock user and open an empty thread with an editable composer. */
export async function openChat(page: Page, userId: string): Promise<WebTestElement> {
  await bootAuthenticatedPage(page, userId, '/chat');
  await dismissWalkthroughIfPresent(page);
  const input = webElements(page).byTestId('chat-message-input');
  await expect.poll(() => input.isVisible()).toBe(true);
  await createNewThread(page);
  await expect.poll(() => input.isVisible()).toBe(true);
  await input.clearComposer();
  return input;
}

/** Wait for the local core socket before asserting that Send is enabled. */
export async function waitForSocketConnected(page: Page): Promise<void> {
  await expect
    .poll(() =>
      page.evaluate(() => {
        const store = (
          window as unknown as {
            __OPENHUMAN_STORE__?: {
              getState: () => { socket?: { byUser?: Record<string, { status?: string }> } };
            };
          }
        ).__OPENHUMAN_STORE__;
        return Object.values(store?.getState().socket?.byUser ?? {}).some(
          entry => entry.status === 'connected'
        );
      })
    )
    .toBe(true);
}

/** Start a thread through the shared UI helpers and wait for its route and composer. */
export async function createNewThread(page: Page): Promise<void> {
  const before = page.url();
  const sidebarButton = webElements(page).byTestId('new-thread-sidebar-button');
  if (await sidebarButton.isVisible().catch(() => false)) {
    await sidebarButton.click();
  } else {
    await webElements(page).byTestId('new-thread-button').click();
  }
  await expect.poll(() => page.url()).not.toBe(before);
  await expect.poll(() => webElements(page).byTestId('chat-message-input').isVisible()).toBe(true);
}
