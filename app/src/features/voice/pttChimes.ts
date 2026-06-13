/**
 * pttChimes — short audio cue playback for push-to-talk session boundaries.
 *
 * The three WAVs (open/close/error) live in `app/src/assets/audio/`. Vite
 * resolves binary assets imported with a string URL out-of-the-box, so a
 * standard `import openSrc from '...wav'` returns a URL the browser can fetch.
 *
 * HTMLAudioElement instances are cached per kind so repeat playback doesn't
 * re-decode the WAV on every press. `play()` may reject under the autoplay
 * policy (no user gesture yet) — we swallow that since PTT is triggered by
 * a global hotkey, not a click, and the chime is non-critical.
 */
import closeSrc from '../../assets/audio/ptt-close.wav';
import errorSrc from '../../assets/audio/ptt-error.wav';
import openSrc from '../../assets/audio/ptt-open.wav';

export type ChimeKind = 'open' | 'close' | 'error';

const sources: Record<ChimeKind, string> = { open: openSrc, close: closeSrc, error: errorSrc };

const cache: Partial<Record<ChimeKind, HTMLAudioElement>> = {};

function getElement(kind: ChimeKind): HTMLAudioElement {
  const cached = cache[kind];
  if (cached) return cached;
  const el = new window.Audio(sources[kind]);
  el.preload = 'auto';
  cache[kind] = el;
  return el;
}

export async function playPttChime(kind: ChimeKind): Promise<void> {
  try {
    const el = getElement(kind);
    el.currentTime = 0;
    await el.play();
  } catch (err) {
    // Autoplay policy can reject silently for the first chime if no gesture
    // has been observed. PTT is non-critical UX feedback so we just log.
    console.debug('[ptt-chime] play failed', { kind, err: String(err) });
  }
}
