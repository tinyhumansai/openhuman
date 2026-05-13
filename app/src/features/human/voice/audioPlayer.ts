/**
 * Lightweight base64 → playable HTMLAudio wrapper. We don't need WebAudio
 * graph here; the viseme scheduler reads `currentTime` directly.
 */

/**
 * Sentinel thrown by the `ended` promise when `stop()` is called. Callers that
 * intentionally cancel playback (e.g. swapping to a new utterance) can detect
 * this and silence the rejection without masking real audio decoder errors.
 */
export class AudioStoppedError extends Error {
  readonly stopped = true;
  constructor() {
    super('stopped');
    this.name = 'AudioStoppedError';
  }
}

export function isAudioStopped(err: unknown): err is AudioStoppedError {
  if (err instanceof AudioStoppedError) return true;
  return typeof err === 'object' && err !== null && (err as { stopped?: unknown }).stopped === true;
}

/**
 * Use as `.catch(swallowAudioStop)` on an orphan `handle.ended` promise, or
 * `catch (err) { swallowAudioStop(err); }` around an awaited one. Swallows the
 * stop sentinel; rethrows anything else so real decoder errors stay visible.
 */
export function swallowAudioStop(err: unknown): void {
  if (isAudioStopped(err)) return;
  throw err;
}

export interface PlaybackHandle {
  /** ms elapsed since audio started. Returns -1 after playback ends. */
  currentMs(): number;
  /**
   * Total audio duration in ms. Returns 0 if `loadedmetadata` has not fired
   * yet — call again after a tick or wait on `metadataReady`. A function (not
   * a static field) so callers always read the latest value rather than a
   * stale snapshot taken before the decoder finished probing.
   */
  durationMs(): number;
  /** Resolves once the decoder reports duration (or the safety timeout fires). */
  metadataReady: Promise<void>;
  /** Stop playback and release the blob URL. Idempotent. */
  stop(): void;
  /** Resolves when the audio finishes naturally. Rejects if `stop()` is called. */
  ended: Promise<void>;
}

export async function playBase64Audio(
  base64: string,
  mime: string = 'audio/mpeg'
): Promise<PlaybackHandle> {
  const bytes = Uint8Array.from(atob(base64), c => c.charCodeAt(0));
  const blob = new Blob([bytes], { type: mime });
  const url = URL.createObjectURL(blob);
  const audio = new window.Audio(url);
  audio.preload = 'auto';

  let stopped = false;
  let endedNaturally = false;
  let resolveEnded!: () => void;
  let rejectEnded!: (err: Error) => void;
  const ended = new Promise<void>((res, rej) => {
    resolveEnded = res;
    rejectEnded = rej;
  });

  const cleanup = () => {
    URL.revokeObjectURL(url);
  };

  audio.addEventListener('ended', () => {
    endedNaturally = true;
    cleanup();
    resolveEnded();
  });
  audio.addEventListener('error', () => {
    cleanup();
    rejectEnded(new Error('audio playback error'));
  });

  // Track metadata readiness without awaiting before `play()`: CEF/Chromium's
  // autoplay policy keys off the synchronous gesture chain, and any `await`
  // between the originating user click and `audio.play()` invalidates it,
  // causing play() to reject with "the user didn't interact with the document
  // first." We capture duration in a side listener and let the caller wait
  // on `metadataReady` separately if it needs it.
  let resolveMetadata!: () => void;
  const metadataReady = new Promise<void>(res => {
    resolveMetadata = res;
  });
  audio.addEventListener(
    'loadedmetadata',
    () => {
      resolveMetadata();
    },
    { once: true }
  );
  // Safety timeout so the procedural-viseme fallback never blocks forever if
  // the decoder skips `loadedmetadata` (some MP3 streams) — fall through to
  // the text-length estimate path in that case.
  window.setTimeout(() => resolveMetadata(), 500);

  try {
    await audio.play();
  } catch (err) {
    cleanup();
    rejectEnded(err instanceof Error ? err : new Error(String(err)));
    throw err;
  }

  return {
    currentMs: () => (endedNaturally || stopped ? -1 : audio.currentTime * 1000),
    durationMs: () => (Number.isFinite(audio.duration) ? audio.duration * 1000 : 0),
    metadataReady,
    stop: () => {
      if (stopped) return;
      stopped = true;
      audio.pause();
      cleanup();
      rejectEnded(new AudioStoppedError());
    },
    ended,
  };
}
