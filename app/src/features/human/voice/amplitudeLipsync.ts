/**
 * Amplitude-driven lip-sync for the realtime voice session (#5399).
 *
 * The classic tap-and-speak path animates the mouth from a viseme *timeline*:
 * frames of `{viseme, ms}` sampled against our own audio element's clock. That
 * is not available here — the realtime SDK owns playback, so there is no
 * `currentMs()` to sample and no per-character timing unless we subscribe to
 * alignment events and reconstruct one.
 *
 * What the SDK does expose is the output signal itself (`getOutputVolume()`),
 * so the mouth is driven from loudness instead. That is genuinely less accurate
 * — it opens and closes with the envelope rather than forming phonemes, so no
 * `M`/`F` closures — but it is in sync *by construction*, because it is the
 * audio being played rather than a prediction of it. A frozen mouth while the
 * agent talks reads as broken; an approximate one reads as alive.
 *
 * The viseme-timeline version is the follow-up, and this is the fallback it
 * would keep for the case where alignment is absent or has run dry.
 */

/** Viseme codes, ordered by how open the mouth is. */
const REST = 'sil';
const NARROW = 'I'; // openness 0.30
const MID = 'E'; // openness 0.45
const OPEN = 'aa'; // openness ~0.95

/**
 * Below this the signal is room tone or the tail of a word, not speech. Holding
 * the mouth open through those gaps is what makes naive amplitude lip-sync look
 * slack-jawed, so anything under it rests.
 */
const SILENCE_FLOOR = 0.04;

/** Where the mouth steps from narrow to mid, and from mid to wide open. */
const MID_THRESHOLD = 0.12;
const OPEN_THRESHOLD = 0.28;

/**
 * Smoothing applied to the raw reading, as the weight given to the new sample.
 *
 * `getOutputVolume()` is sampled per animation frame and is noisy at that rate:
 * fed straight through it produces a chattering mouth that reads as a glitch
 * rather than as speech. Asymmetric on purpose — opening tracks the signal
 * quickly so consonant onsets land on time, closing lags so the mouth does not
 * snap shut inside a word.
 */
const ATTACK = 0.55;
const RELEASE = 0.18;

/** Smooth one amplitude sample toward the previous level. Pure + unit-tested. */
export function smoothAmplitude(previous: number, sample: number): number {
  const weight = sample > previous ? ATTACK : RELEASE;
  return previous + (sample - previous) * weight;
}

/**
 * Map a smoothed amplitude (0..1) onto a viseme code. Steps rather than
 * interpolates because the Rive mouth is driven by a code, not a scalar.
 * Pure + unit-tested.
 */
export function amplitudeToVisemeCode(level: number): string {
  if (!Number.isFinite(level) || level < SILENCE_FLOOR) return REST;
  if (level < MID_THRESHOLD) return NARROW;
  if (level < OPEN_THRESHOLD) return MID;
  return OPEN;
}

/**
 * What the realtime controls publish for the mascot to read. Held in a ref and
 * mutated in place: the mascot samples it once per animation frame, and routing
 * a 60fps signal through React state would re-render the page on every frame.
 */
export interface RealtimeVoiceAudio {
  /** SDK accessor for output loudness, or null when no session is live. */
  getOutputVolume: (() => number) | null;
  /** Whether the agent is currently speaking (SDK `isSpeaking`). */
  speaking: boolean;
}

export const IDLE_REALTIME_VOICE_AUDIO: RealtimeVoiceAudio = {
  getOutputVolume: null,
  speaking: false,
};
