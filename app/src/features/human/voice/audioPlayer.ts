/**
 * Lightweight base64 → playable HTMLAudio wrapper. We don't need WebAudio
 * graph here; the viseme scheduler reads `currentTime` directly.
 */
export interface PlaybackHandle {
  /** ms elapsed since audio started. Returns -1 after playback ends. */
  currentMs(): number;
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

  try {
    await audio.play();
  } catch (err) {
    cleanup();
    rejectEnded(err instanceof Error ? err : new Error(String(err)));
    throw err;
  }

  return {
    currentMs: () => (endedNaturally || stopped ? -1 : audio.currentTime * 1000),
    stop: () => {
      if (stopped) return;
      stopped = true;
      audio.pause();
      cleanup();
      rejectEnded(new Error('stopped'));
    },
    ended,
  };
}
