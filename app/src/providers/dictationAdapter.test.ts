import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { VoiceNotCompiledError } from '../features/human/voice/sttClient';
import {
  createOpenHumanDictationAdapter,
  DICTATION_START_TIMEOUT_MS,
  DICTATION_STOP_TIMEOUT_MS,
  DICTATION_TRANSCRIPTION_TIMEOUT_MS,
  isDictationCaptureSupported,
  MAX_DICTATION_RECORDING_MS,
  type OpenHumanDictationAdapter,
} from './dictationAdapter';

const transcribe = vi.fn();
const encodeWav = vi.fn();
vi.mock('../features/human/voice/wavEncoder', () => ({
  encodeBlobToWav: (...args: unknown[]) => encodeWav(...args),
}));
vi.mock('../features/human/voice/sttClient', async importOriginal => ({
  ...(await importOriginal<typeof import('../features/human/voice/sttClient')>()),
  transcribeWithFactory: (...args: unknown[]) => transcribe(...args),
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

class FakeTrack extends EventTarget {
  readyState: MediaStreamTrackState = 'live';
  stop = vi.fn(() => {
    this.readyState = 'ended';
  });
}

type FakeRecorder = {
  state: RecordingState;
  mimeType: string;
  ondataavailable: ((event: BlobEvent) => void) | null;
  onstop: (() => void) | null;
  onerror: (() => void) | null;
  start: ReturnType<typeof vi.fn>;
  stop: ReturnType<typeof vi.fn>;
};

async function flush() {
  for (let step = 0; step < 6; step++) await Promise.resolve();
}

describe('OpenHumanDictationAdapter', () => {
  let adapter: OpenHumanDictationAdapter;
  let track: FakeTrack;
  let stream: MediaStream;
  let getUserMedia: ReturnType<typeof vi.fn>;
  let recorder: FakeRecorder;
  let constructorError: boolean;
  let startError: boolean;
  let stopError: boolean;
  let hangStop: boolean;
  let emptyAudio: boolean;
  let actualMime: string | undefined;
  let recorderOptions: MediaRecorderOptions | undefined;
  let originalMediaDevices: PropertyDescriptor | undefined;

  beforeEach(() => {
    vi.useFakeTimers();
    transcribe.mockReset().mockResolvedValue('spoken words');
    encodeWav.mockReset().mockResolvedValue(new Blob(['wav audio'], { type: 'audio/wav' }));
    constructorError = false;
    startError = false;
    stopError = false;
    hangStop = false;
    emptyAudio = false;
    actualMime = undefined;
    recorderOptions = undefined;
    track = new FakeTrack();
    stream = { getTracks: () => [track] } as unknown as MediaStream;
    getUserMedia = vi.fn().mockResolvedValue(stream);
    originalMediaDevices = Object.getOwnPropertyDescriptor(navigator, 'mediaDevices');
    Object.defineProperty(navigator, 'mediaDevices', {
      value: { getUserMedia },
      configurable: true,
    });
    class Recorder {
      constructor(_stream: MediaStream, options?: MediaRecorderOptions) {
        if (constructorError) throw new Error('private constructor detail');
        recorderOptions = options;
        recorder = {
          state: 'inactive',
          mimeType: actualMime ?? options?.mimeType ?? 'audio/webm',
          ondataavailable: null,
          onstop: null,
          onerror: null,
          start: vi.fn(() => {
            if (startError) throw new Error('private start detail');
            recorder.state = 'recording';
          }),
          stop: vi.fn(() => {
            if (stopError) throw new Error('private stop detail');
            recorder.state = 'inactive';
            if (hangStop) return;
            if (!emptyAudio) {
              recorder.ondataavailable?.({
                data: new Blob([new Uint8Array([1, 2, 3])], { type: recorder.mimeType }),
              } as BlobEvent);
            }
            recorder.onstop?.();
          }),
        };
        return recorder as unknown as MediaRecorder;
      }
      static isTypeSupported(mime: string) {
        return mime.startsWith('audio/mp4');
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

  function listen() {
    const session = adapter.listen();
    const speech = vi.fn();
    const ended = vi.fn();
    const started = vi.fn();
    session.onSpeech(speech);
    session.onSpeechEnd(ended);
    session.onSpeechStart(started);
    return { session, speech, ended, started };
  }

  it('records and emits one final transcript before stop resolves, leaving input editable', async () => {
    const result = deferred<string>();
    transcribe.mockReturnValue(result.promise);
    const h = listen();
    expect(adapter.disableInputDuringDictation).toBe(false);
    expect(adapter.getSnapshot()).toEqual({ phase: 'starting', error: null });
    await flush();
    expect(adapter.getSnapshot().phase).toBe('recording');
    expect(h.started).toHaveBeenCalledOnce();
    expect(getUserMedia).toHaveBeenCalledWith({
      audio: expect.objectContaining({ channelCount: 1, echoCancellation: true }),
    });

    const settled = vi.fn();
    const stopping = h.session.stop().then(settled);
    expect(adapter.getSnapshot().phase).toBe('transcribing');
    expect(track.stop).toHaveBeenCalledOnce();
    expect(settled).not.toHaveBeenCalled();
    expect(h.speech).not.toHaveBeenCalled();

    result.resolve('  spoken words  ');
    await stopping;
    expect(h.speech).toHaveBeenCalledExactlyOnceWith({ transcript: 'spoken words', isFinal: true });
    expect(h.speech.mock.invocationCallOrder[0]).toBeLessThan(settled.mock.invocationCallOrder[0]);
    expect(h.ended).toHaveBeenCalledOnce();
    expect(h.session.status).toEqual({ type: 'ended', reason: 'stopped' });
    expect(adapter.getSnapshot()).toEqual({ phase: 'idle', error: null });
  });

  it('uses the actual recorder MIME for the blob sent to configured STT', async () => {
    actualMime = 'audio/mp4';
    const h = listen();
    await flush();
    expect(recorderOptions).toEqual({
      audioBitsPerSecond: 128_000,
      mimeType: 'audio/mp4;codecs=mp4a.40.2',
    });
    await h.session.stop();
    expect(transcribe).toHaveBeenCalledOnce();
    const blob = transcribe.mock.calls[0][0] as Blob;
    expect(blob.type).toBe('audio/mp4');
    expect(blob.size).toBe(3);
  });

  it('lets the browser choose a codec when none of the preferred MIME types is supported', async () => {
    vi.spyOn(MediaRecorder, 'isTypeSupported').mockReturnValue(false);
    const h = listen();
    await flush();
    expect(recorderOptions?.mimeType).toBeUndefined();
    await h.session.stop();
    expect((transcribe.mock.calls[0][0] as Blob).type).toBe('audio/webm');
  });

  it.each([
    ['NotAllowedError', 'permission-denied'],
    ['SecurityError', 'permission-denied'],
    ['NotFoundError', 'device-unavailable'],
    ['OverconstrainedError', 'device-unavailable'],
    ['NotReadableError', 'device-in-use'],
    ['UnexpectedError', 'microphone-unavailable'],
  ])('ends and reports a content-free error for %s', async (name, code) => {
    getUserMedia.mockRejectedValue(new DOMException('secret device detail', name));
    const h = listen();
    await flush();
    expect(adapter.getSnapshot()).toEqual({ phase: 'idle', error: code });
    expect(h.session.status).toEqual({ type: 'ended', reason: 'error' });
    expect(h.ended).toHaveBeenCalledOnce();
    expect(h.speech).not.toHaveBeenCalled();
    expect(transcribe).not.toHaveBeenCalled();
    expect(JSON.stringify(adapter.getSnapshot())).not.toContain('secret');
  });

  it.each(['stop', 'cancel', 'dispose'] as const)(
    '%s during a permission prompt discards the late stream',
    async operation => {
      const permission = deferred<MediaStream>();
      getUserMedia.mockReturnValue(permission.promise);
      const h = listen();
      await flush();
      if (operation === 'stop') await h.session.stop();
      else adapter[operation]();
      expect(h.session.status).toEqual({ type: 'ended', reason: 'cancelled' });
      expect(h.ended).toHaveBeenCalledOnce();
      permission.resolve(stream);
      await flush();
      expect(track.stop).toHaveBeenCalledOnce();
      expect(h.speech).not.toHaveBeenCalled();
      expect(transcribe).not.toHaveBeenCalled();
      expect(adapter.getSnapshot().phase).toBe('idle');
    }
  );

  it('cancels before capture starts without requesting permission', async () => {
    const h = listen();
    h.session.cancel();
    await flush();
    expect(getUserMedia).not.toHaveBeenCalled();
    expect(h.ended).toHaveBeenCalledOnce();
  });

  it.each(['cancel', 'dispose'] as const)(
    '%s while STT is pending prevents late text and returns immediately to idle',
    async operation => {
      const result = deferred<string>();
      transcribe.mockReturnValue(result.promise);
      const h = listen();
      await flush();
      const stopping = h.session.stop();
      adapter[operation]();
      await stopping;
      expect(adapter.getSnapshot()).toEqual({ phase: 'idle', error: null });
      expect(h.session.status).toEqual({ type: 'ended', reason: 'cancelled' });
      result.resolve('late transcript');
      await flush();
      expect(h.speech).not.toHaveBeenCalled();
      expect(h.ended).toHaveBeenCalledOnce();
      expect(recorder.onstop).toBeNull();
      expect(recorder.ondataavailable).toBeNull();
      expect(recorder.onerror).toBeNull();
    }
  );

  it('suppresses queued recorder events after cancel', async () => {
    const h = listen();
    await flush();
    const lateData = recorder.ondataavailable!;
    const lateStop = recorder.onstop!;
    const lateError = recorder.onerror!;
    adapter.cancel();
    lateData({ data: new Blob(['late audio']) } as BlobEvent);
    lateStop();
    lateError();
    await flush();
    expect(transcribe).not.toHaveBeenCalled();
    expect(h.speech).not.toHaveBeenCalled();
    expect(h.ended).toHaveBeenCalledOnce();
    expect(track.stop).toHaveBeenCalledOnce();
  });

  it('ends on microphone device loss and discards recorded data', async () => {
    const h = listen();
    await flush();
    track.readyState = 'ended';
    track.dispatchEvent(new Event('ended'));
    expect(adapter.getSnapshot()).toEqual({ phase: 'idle', error: 'device-unavailable' });
    expect(h.ended).toHaveBeenCalledOnce();
    expect(track.stop).toHaveBeenCalledOnce();
    expect(recorder.onstop).toBeNull();
    expect(transcribe).not.toHaveBeenCalled();
  });

  it('rejects an already ended track before recording', async () => {
    track.readyState = 'ended';
    const h = listen();
    await flush();
    expect(adapter.getSnapshot().error).toBe('device-unavailable');
    expect(h.started).not.toHaveBeenCalled();
    expect(track.stop).toHaveBeenCalledOnce();
  });

  it.each(['constructor', 'start', 'stop', 'event'] as const)(
    'cleans the microphone and ends on recorder %s failure',
    async failure => {
      constructorError = failure === 'constructor';
      startError = failure === 'start';
      stopError = failure === 'stop';
      const h = listen();
      await flush();
      if (failure === 'stop') await h.session.stop();
      if (failure === 'event') recorder.onerror!();
      expect(adapter.getSnapshot()).toEqual({ phase: 'idle', error: 'recorder-failed' });
      expect(h.ended).toHaveBeenCalledOnce();
      expect(track.stop).toHaveBeenCalledOnce();
      expect(h.speech).not.toHaveBeenCalled();
      expect(transcribe).not.toHaveBeenCalled();
    }
  );

  it('double stop shares completion and transcribes only once', async () => {
    const result = deferred<string>();
    transcribe.mockReturnValue(result.promise);
    const h = listen();
    await flush();
    const first = h.session.stop();
    const second = h.session.stop();
    expect(first).toBe(second);
    expect(recorder.stop).toHaveBeenCalledOnce();
    await flush();
    expect(transcribe).toHaveBeenCalledOnce();
    result.resolve('completed');
    await first;
    await h.session.stop();
    expect(h.speech).toHaveBeenCalledOnce();
    expect(h.ended).toHaveBeenCalledOnce();
  });

  it('discards a timed-out recording without uploading audio or changing the draft', async () => {
    const h = listen();
    await flush();
    await vi.advanceTimersByTimeAsync(MAX_DICTATION_RECORDING_MS);
    expect(h.speech).not.toHaveBeenCalled();
    expect(transcribe).not.toHaveBeenCalled();
    expect(encodeWav).not.toHaveBeenCalled();
    expect(track.stop).toHaveBeenCalledOnce();
    expect(adapter.getSnapshot()).toEqual({ phase: 'idle', error: 'timed-out' });
    expect(h.ended).toHaveBeenCalledExactlyOnceWith({ transcript: '', isFinal: true });
    await h.session.stop();
    expect(transcribe).not.toHaveBeenCalled();
  });

  it('bounds a hanging permission request and discards a later grant', async () => {
    const permission = deferred<MediaStream>();
    getUserMedia.mockReturnValue(permission.promise);
    const h = listen();
    await flush();
    await vi.advanceTimersByTimeAsync(DICTATION_START_TIMEOUT_MS);
    expect(adapter.getSnapshot().error).toBe('timed-out');
    expect(h.ended).toHaveBeenCalledOnce();
    permission.resolve(stream);
    await flush();
    expect(track.stop).toHaveBeenCalledOnce();
    expect(h.speech).not.toHaveBeenCalled();
  });

  it('bounds a recorder that never emits stop and releases its stream promptly', async () => {
    hangStop = true;
    const h = listen();
    await flush();
    const stopping = h.session.stop();
    expect(track.stop).toHaveBeenCalledOnce();
    await vi.advanceTimersByTimeAsync(DICTATION_STOP_TIMEOUT_MS);
    await stopping;
    expect(adapter.getSnapshot().error).toBe('timed-out');
    expect(h.ended).toHaveBeenCalledOnce();
    expect(transcribe).not.toHaveBeenCalled();
  });

  it('bounds STT and ignores a reply after the bound expires', async () => {
    const result = deferred<string>();
    transcribe.mockReturnValue(result.promise);
    const h = listen();
    await flush();
    const stopping = h.session.stop();
    await vi.advanceTimersByTimeAsync(DICTATION_TRANSCRIPTION_TIMEOUT_MS);
    await stopping;
    expect(adapter.getSnapshot().error).toBe('timed-out');
    result.resolve('expired text');
    await flush();
    expect(h.speech).not.toHaveBeenCalled();
    expect(h.ended).toHaveBeenCalledOnce();
  });

  it.each([
    ['voice-unavailable', new VoiceNotCompiledError()],
    ['transcription-failed', new Error('private backend detail')],
  ])('surfaces %s without raw error text', async (code, error) => {
    transcribe.mockRejectedValue(error);
    const h = listen();
    await flush();
    await h.session.stop();
    expect(adapter.getSnapshot()).toEqual({ phase: 'idle', error: code });
    expect(h.speech).not.toHaveBeenCalled();
    expect(h.ended).toHaveBeenCalledOnce();
  });

  it.each(['no-audio', 'no-speech'] as const)('ends cleanly when there is %s', async code => {
    emptyAudio = code === 'no-audio';
    transcribe.mockResolvedValue('  ');
    const h = listen();
    await flush();
    await h.session.stop();
    expect(adapter.getSnapshot().error).toBe(code);
    expect(h.speech).not.toHaveBeenCalled();
    expect(h.ended).toHaveBeenCalledOnce();
    if (emptyAudio) expect(transcribe).not.toHaveBeenCalled();
  });

  it('replacing a session invalidates its pending transcript', async () => {
    const result = deferred<string>();
    transcribe.mockReturnValue(result.promise);
    const first = listen();
    await flush();
    const stopping = first.session.stop();
    const replacementTrack = new FakeTrack();
    getUserMedia.mockResolvedValue({ getTracks: () => [replacementTrack] });
    const second = listen();
    await flush();
    result.resolve('old text');
    await stopping;
    expect(first.speech).not.toHaveBeenCalled();
    expect(second.speech).not.toHaveBeenCalled();
    expect(adapter.getSnapshot().phase).toBe('recording');
    adapter.cancel();
    expect(second.ended).toHaveBeenCalledOnce();
  });

  it('publishes stable snapshots and unsubscribes observers', async () => {
    const subscriber = vi.fn();
    const unsubscribe = adapter.subscribe(subscriber);
    expect(adapter.getSnapshot()).toBe(adapter.getSnapshot());
    const h = listen();
    await flush();
    expect(subscriber).toHaveBeenCalledTimes(2);
    unsubscribe();
    await h.session.stop();
    expect(subscriber).toHaveBeenCalledTimes(2);
  });

  it('supports listening again after disposal for effect re-setup', async () => {
    adapter.dispose();
    const h = listen();
    await flush();
    expect(adapter.getSnapshot().phase).toBe('recording');
    adapter.cancel();
    expect(h.ended).toHaveBeenCalledOnce();
  });

  it('does not let a throwing subscriber prevent cleanup or completion', async () => {
    const h = listen();
    h.session.onSpeech(() => {
      throw new Error('subscriber failed');
    });
    h.session.onSpeechEnd(() => {
      throw new Error('subscriber failed');
    });
    adapter.subscribe(() => {
      throw new Error('subscriber failed');
    });
    await flush();
    await h.session.stop();
    expect(track.stop).toHaveBeenCalledOnce();
    expect(h.ended).toHaveBeenCalledOnce();
    expect(adapter.getSnapshot().phase).toBe('idle');
  });

  it('detects unavailable capture without requesting permission', async () => {
    vi.stubGlobal('MediaRecorder', undefined);
    expect(isDictationCaptureSupported()).toBe(false);
    const h = listen();
    await flush();
    expect(adapter.getSnapshot().error).toBe('microphone-unavailable');
    expect(h.ended).toHaveBeenCalledOnce();
    expect(getUserMedia).not.toHaveBeenCalled();
  });
});
