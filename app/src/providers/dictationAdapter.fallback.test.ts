import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { VoiceNotCompiledError } from '../features/human/voice/sttClient';
import {
  createOpenHumanDictationAdapter,
  DICTATION_TRANSCRIPTION_TIMEOUT_MS,
  type OpenHumanDictationAdapter,
} from './dictationAdapter';

const transcribe = vi.fn();
const encodeWav = vi.fn();
vi.mock('../features/human/voice/sttClient', async importOriginal => ({
  ...(await importOriginal<typeof import('../features/human/voice/sttClient')>()),
  transcribeWithFactory: (...args: unknown[]) => transcribe(...args),
}));
vi.mock('../features/human/voice/wavEncoder', () => ({
  encodeBlobToWav: (...args: unknown[]) => encodeWav(...args),
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

async function flush() {
  for (let step = 0; step < 6; step++) await Promise.resolve();
}

describe('dictation WAV compatibility fallback', () => {
  let adapter: OpenHumanDictationAdapter;
  let originalMediaDevices: PropertyDescriptor | undefined;
  let wav: Blob;
  let track: EventTarget & { stop: ReturnType<typeof vi.fn>; readyState: string };

  beforeEach(() => {
    vi.useFakeTimers();
    wav = new Blob(['portable PCM'], { type: 'audio/wav' });
    transcribe.mockReset().mockResolvedValue('dictated words');
    encodeWav.mockReset().mockResolvedValue(wav);
    track = Object.assign(new EventTarget(), { stop: vi.fn(), readyState: 'live' });
    originalMediaDevices = Object.getOwnPropertyDescriptor(navigator, 'mediaDevices');
    Object.defineProperty(navigator, 'mediaDevices', {
      configurable: true,
      value: { getUserMedia: vi.fn().mockResolvedValue({ getTracks: () => [track] }) },
    });
    class Recorder {
      state: RecordingState = 'inactive';
      mimeType = 'audio/webm';
      ondataavailable: ((event: BlobEvent) => void) | null = null;
      onstop: (() => void) | null = null;
      onerror: (() => void) | null = null;
      start() {
        this.state = 'recording';
      }
      stop() {
        this.state = 'inactive';
        this.ondataavailable?.({
          data: new Blob(['native audio'], { type: this.mimeType }),
        } as BlobEvent);
        this.onstop?.();
      }
      static isTypeSupported() {
        return false;
      }
    }
    vi.stubGlobal('MediaRecorder', Recorder);
    adapter = createOpenHumanDictationAdapter();
  });

  afterEach(() => {
    adapter.dispose();
    if (originalMediaDevices) {
      Object.defineProperty(navigator, 'mediaDevices', originalMediaDevices);
    } else {
      delete (navigator as { mediaDevices?: MediaDevices }).mediaDevices;
    }
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  async function listen() {
    const session = adapter.listen();
    const speech = vi.fn();
    const ended = vi.fn();
    session.onSpeech(speech);
    session.onSpeechEnd(ended);
    await flush();
    return { session, speech, ended };
  }

  it('keeps a successful native recording on one STT call with no conversion', async () => {
    const h = await listen();
    await h.session.stop();
    expect(transcribe).toHaveBeenCalledOnce();
    expect((transcribe.mock.calls[0][0] as Blob).type).toBe('audio/webm');
    expect(encodeWav).not.toHaveBeenCalled();
    expect(h.speech).toHaveBeenCalledExactlyOnceWith({
      transcript: 'dictated words',
      isFinal: true,
    });
  });

  it('retries one rejected native clip as portable PCM WAV and emits the fallback text', async () => {
    transcribe.mockRejectedValueOnce(new Error('native container rejected'));
    const h = await listen();
    await h.session.stop();
    expect(transcribe).toHaveBeenCalledTimes(2);
    expect(encodeWav).toHaveBeenCalledExactlyOnceWith(transcribe.mock.calls[0][0]);
    expect(transcribe.mock.calls[1][0]).toBe(wav);
    expect(h.speech).toHaveBeenCalledExactlyOnceWith({
      transcript: 'dictated words',
      isFinal: true,
    });
    expect(adapter.getSnapshot()).toEqual({ phase: 'idle', error: null });
    expect(track.stop).toHaveBeenCalledOnce();
  });

  it('ends after the single fallback STT call fails, without exposing raw error details', async () => {
    transcribe.mockRejectedValue(new Error('private provider detail'));
    const h = await listen();
    await h.session.stop();
    expect(transcribe).toHaveBeenCalledTimes(2);
    expect(encodeWav).toHaveBeenCalledOnce();
    expect(adapter.getSnapshot()).toEqual({ phase: 'idle', error: 'transcription-failed' });
    expect(h.speech).not.toHaveBeenCalled();
    expect(h.ended).toHaveBeenCalledOnce();
  });

  it('ends if WAV conversion fails and makes no second STT request', async () => {
    transcribe.mockRejectedValueOnce(new Error('native container rejected'));
    encodeWav.mockRejectedValueOnce(new Error('decode failed'));
    const h = await listen();
    await h.session.stop();
    expect(transcribe).toHaveBeenCalledOnce();
    expect(adapter.getSnapshot()).toEqual({ phase: 'idle', error: 'transcription-failed' });
    expect(h.speech).not.toHaveBeenCalled();
    expect(h.ended).toHaveBeenCalledOnce();
  });

  it('does not convert or retry when the core has no voice dispatch', async () => {
    transcribe.mockRejectedValueOnce(new VoiceNotCompiledError());
    const h = await listen();
    await h.session.stop();
    expect(transcribe).toHaveBeenCalledOnce();
    expect(encodeWav).not.toHaveBeenCalled();
    expect(adapter.getSnapshot()).toEqual({ phase: 'idle', error: 'voice-unavailable' });
  });

  it('cancellation during conversion suppresses the second STT request', async () => {
    const conversion = deferred<Blob>();
    transcribe.mockRejectedValueOnce(new Error('native container rejected'));
    encodeWav.mockReturnValue(conversion.promise);
    const h = await listen();
    const stopping = h.session.stop();
    await flush();
    expect(encodeWav).toHaveBeenCalledOnce();
    adapter.cancel();
    await stopping;
    conversion.resolve(wav);
    await flush();
    expect(transcribe).toHaveBeenCalledOnce();
    expect(h.speech).not.toHaveBeenCalled();
    expect(h.ended).toHaveBeenCalledOnce();
    expect(adapter.getSnapshot()).toEqual({ phase: 'idle', error: null });
  });

  it('cancellation during fallback STT suppresses its late transcript', async () => {
    const fallback = deferred<string>();
    transcribe
      .mockRejectedValueOnce(new Error('native container rejected'))
      .mockReturnValueOnce(fallback.promise);
    const h = await listen();
    const stopping = h.session.stop();
    await flush();
    expect(transcribe).toHaveBeenCalledTimes(2);
    adapter.cancel();
    await stopping;
    fallback.resolve('late fallback text');
    await flush();
    expect(h.speech).not.toHaveBeenCalled();
    expect(h.ended).toHaveBeenCalledOnce();
    expect(adapter.getSnapshot()).toEqual({ phase: 'idle', error: null });
  });

  it('uses one total transcription deadline across the native call, conversion, and retry', async () => {
    const native = deferred<string>();
    const conversion = deferred<Blob>();
    const fallback = deferred<string>();
    transcribe.mockReturnValueOnce(native.promise).mockReturnValueOnce(fallback.promise);
    encodeWav.mockReturnValue(conversion.promise);
    const h = await listen();
    const stopping = h.session.stop();
    await vi.advanceTimersByTimeAsync(20_000);
    native.reject(new Error('native container rejected'));
    await flush();
    await vi.advanceTimersByTimeAsync(20_000);
    conversion.resolve(wav);
    await flush();
    expect(transcribe).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(DICTATION_TRANSCRIPTION_TIMEOUT_MS - 40_001);
    expect(adapter.getSnapshot().phase).toBe('transcribing');
    await vi.advanceTimersByTimeAsync(1);
    await stopping;
    expect(adapter.getSnapshot()).toEqual({ phase: 'idle', error: 'timed-out' });
    fallback.resolve('expired fallback text');
    await flush();
    expect(h.speech).not.toHaveBeenCalled();
    expect(h.ended).toHaveBeenCalledOnce();
  });

  it('a conversion that outlasts the deadline never starts a fallback request', async () => {
    const conversion = deferred<Blob>();
    transcribe.mockRejectedValueOnce(new Error('native container rejected'));
    encodeWav.mockReturnValue(conversion.promise);
    const h = await listen();
    const stopping = h.session.stop();
    await flush();
    await vi.advanceTimersByTimeAsync(DICTATION_TRANSCRIPTION_TIMEOUT_MS);
    await stopping;
    conversion.resolve(wav);
    await flush();
    expect(transcribe).toHaveBeenCalledOnce();
    expect(h.speech).not.toHaveBeenCalled();
    expect(h.ended).toHaveBeenCalledOnce();
    expect(adapter.getSnapshot()).toEqual({ phase: 'idle', error: 'timed-out' });
  });
});
