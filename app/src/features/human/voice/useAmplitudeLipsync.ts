import { type RefObject, useEffect, useRef, useState } from 'react';

import {
  amplitudeToVisemeCode,
  type RealtimeVoiceAudio,
  smoothAmplitude,
} from './amplitudeLipsync';

export interface AmplitudeLipsync {
  /** True while the realtime agent is speaking and driving the mouth. */
  active: boolean;
  /** Viseme code for Rive's `mouthVisemeCode` input. */
  visemeCode: string;
}

/**
 * Drive the mascot's mouth from the realtime session's output loudness.
 *
 * The animation-frame loop exists only while `enabled` is true — the caller
 * passes the agent's speaking edge, so an idle Human tab (and the classic
 * voice path, which never speaks a realtime session) schedules no frames at
 * all. `enabled` gates the effect rather than being read inside the loop
 * because a ref cannot re-subscribe an effect: the on/off edge changes a couple
 * of times per turn, so it is cheap as a dependency, while the amplitude it
 * drives stays in a ref because that moves every frame. Reads the SDK accessor
 * out of a ref rather than props because the session lives inside
 * `RealtimeVoiceControls` (which owns its own `ConversationProvider`) — see
 * `RealtimeVoiceAudio`.
 *
 * State is committed only when the viseme code actually changes. The smoothed
 * level moves every frame, but the code it maps to steps between four values,
 * so re-rendering on the raw level would reconcile the page ~60 times a second
 * to produce the same mouth — the exact cost the chat panel is memoised to
 * avoid (#5357).
 */
export function useAmplitudeLipsync(
  audio: RefObject<RealtimeVoiceAudio>,
  enabled: boolean
): AmplitudeLipsync {
  const [state, setState] = useState<AmplitudeLipsync>({ active: false, visemeCode: 'sil' });
  const levelRef = useRef(0);
  // Mirrors `state` for the loop to compare against without re-subscribing the
  // effect on every change.
  const codeRef = useRef('sil');
  const activeRef = useRef(false);

  useEffect(() => {
    const commit = (active: boolean, visemeCode: string): void => {
      if (active === activeRef.current && visemeCode === codeRef.current) return;
      activeRef.current = active;
      codeRef.current = visemeCode;
      setState({ active, visemeCode });
    };

    // Disabled: schedule nothing, and make sure the mouth is at rest. The loop
    // is the only thing that commits 'sil' when speech stops, and it does not
    // run here, so the reset has to happen on the disabling render instead —
    // otherwise the mouth would freeze on its last shape.
    if (!enabled) {
      levelRef.current = 0;
      commit(false, 'sil');
      return;
    }

    let raf = 0;
    let stopped = false;

    const tick = (): void => {
      if (stopped) return;
      const { getOutputVolume, speaking } = audio.current;
      if (!speaking || !getOutputVolume) {
        levelRef.current = 0;
        commit(false, 'sil');
      } else {
        // The SDK reads from a live analyser, so a session torn down mid-frame
        // makes this throw rather than return 0.
        let sample: number;
        try {
          sample = getOutputVolume();
        } catch {
          sample = Number.NaN;
        }
        if (Number.isFinite(sample)) {
          levelRef.current = smoothAmplitude(levelRef.current, sample);
          commit(true, amplitudeToVisemeCode(levelRef.current));
        } else {
          // A bad reading resets rather than decays, for two reasons. Smoothing
          // toward 0 would hold the mouth open for ~16 frames after the audio is
          // already gone; and smoothing toward a non-finite value poisons
          // `levelRef` permanently — every later sample stays NaN, so the mouth
          // never animates again for the rest of the call. Guarding only at the
          // viseme mapping hides that as a quiet mouth instead of an error.
          levelRef.current = 0;
          commit(false, 'sil');
        }
      }
      raf = window.requestAnimationFrame(tick);
    };

    raf = window.requestAnimationFrame(tick);
    return () => {
      stopped = true;
      window.cancelAnimationFrame(raf);
    };
  }, [audio, enabled]);

  return state;
}
