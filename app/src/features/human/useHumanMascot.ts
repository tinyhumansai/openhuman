import { useEffect, useRef, useState } from 'react';

import { subscribeChatEvents } from '../../services/chatService';
import type { MascotFace } from './Mascot';
import { lerpViseme, type VisemeShape, VISEMES } from './Mascot/visemes';

/** ms the mouth holds the target viseme before decaying back to rest. */
const VISEME_DECAY_MS = 180;

/**
 * Pick a viseme from the trailing letter of a text delta. Heuristic — we
 * have no phoneme data — but it gives the mouth varied motion that tracks
 * the streaming text instead of just opening and closing the same way.
 */
export function pickViseme(delta: string): VisemeShape {
  const ch = delta.replace(/[^a-zA-Z]/g, '').slice(-1).toLowerCase();
  switch (ch) {
    case 'a':
      return VISEMES.A;
    case 'e':
      return VISEMES.E;
    case 'i':
    case 'y':
      return VISEMES.I;
    case 'o':
      return VISEMES.O;
    case 'u':
    case 'w':
      return VISEMES.U;
    case 'm':
    case 'b':
    case 'p':
      return VISEMES.M;
    case 'f':
    case 'v':
      return VISEMES.F;
    default:
      return VISEMES.E;
  }
}

/**
 * Drives the mascot's face/mouth from streaming chat events.
 *
 * - `inference_start` → thinking
 * - `text_delta` → speaking, with the mouth shape picked from the delta and
 *   decaying back toward rest between deltas
 * - `chat_done` / `chat_error` → back to neutral
 */
export function useHumanMascot(): { face: MascotFace; viseme: VisemeShape } {
  const [face, setFace] = useState<MascotFace>('normal');
  const targetRef = useRef<VisemeShape>(VISEMES.REST);
  const lastDeltaAtRef = useRef(0);
  const [, force] = useState(0);

  useEffect(() => {
    const unsub = subscribeChatEvents({
      onInferenceStart: () => setFace('thinking'),
      onTextDelta: e => {
        setFace('speaking');
        targetRef.current = pickViseme(e.delta);
        lastDeltaAtRef.current = performance.now();
      },
      onDone: () => setFace('normal'),
      onError: () => setFace('normal'),
    });
    return unsub;
  }, []);

  useEffect(() => {
    if (face !== 'speaking') return;
    let raf = 0;
    const loop = () => {
      force(t => t + 1);
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
  }, [face]);

  let viseme: VisemeShape = VISEMES.REST;
  if (face === 'speaking') {
    const since = performance.now() - lastDeltaAtRef.current;
    const decay = Math.max(0, Math.min(1, since / VISEME_DECAY_MS));
    viseme = lerpViseme(targetRef.current, VISEMES.REST, decay);
  }

  return { face, viseme };
}
