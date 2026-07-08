/**
 * Resolve per-slot mascot render state for the Meet video frame (issue #4277).
 *
 * The producer (`MascotFrameProducer.tsx`) composites up to two mascots into
 * the outgoing camera frame. This hook is the single source of truth for
 * *which* mascot each slot shows and *what face* it wears, given the live
 * speaking-state event + the meeting phase. Kept as a thin, pure-ish selector
 * hook (redux in → render state out) so the face table is unit-testable via
 * `renderHook` + `preloadedState` without mounting any WebGL.
 *
 * Slot mapping (locked by the mascotSlice contract):
 *   - slot 0 = primary   = `selectedMascotId`
 *   - slot 1 = secondary = `secondaryMascotId`
 * and `activeMascotSlot` on the speaking-state event names which slot is
 * currently speaking the audio participants hear.
 */
import debug from 'debug';

import { useAppSelector } from '../../store/hooks';
import {
  selectDualMascotEnabled,
  selectMeetingMascotVoicePair,
  selectSecondaryMascotId,
  selectSelectedMascotId,
} from '../../store/mascotSlice';
import type { MascotFace } from '../human/Mascot';

const log = debug('meet:mascots');

/**
 * Coarse lifecycle phase of the on-camera mascot(s):
 *   - `greeting` — just joined; both mascots wave hello.
 *   - `active`   — the call is live; faces track who is speaking.
 *   - `signoff`  — the session is tearing down; both wave goodbye.
 */
export type MeetingPhase = 'greeting' | 'active' | 'signoff';

/** Which slot is producing the audio the call currently hears. */
export type ActiveMascotSlot = 0 | 1;

export interface MeetingMascotSlotRender {
  /** Manifest mascot id, or `null` for the primary on the default mascot. */
  mascotId: string | null;
  /** MascotFace to render this tick. */
  face: MascotFace;
}

export interface MeetingMascotsRenderState {
  /** True when a distinct second mascot is enabled (drives dual composite). */
  dualEnabled: boolean;
  primary: MeetingMascotSlotRender;
  /** Non-null only when `dualEnabled`. */
  secondary: MeetingMascotSlotRender | null;
}

export interface UseMeetingMascotsInput {
  /** Live from `meet-video:speaking-state` — is audio currently streaming. */
  speaking: boolean;
  /** Live from `meet-video:speaking-state` — which slot owns that audio. */
  activeMascotSlot: ActiveMascotSlot;
  /** Meeting lifecycle phase; drives the greeting / sign-off wave. */
  phase: MeetingPhase;
}

/**
 * Face the *speaking* slot wears while the call is active: mouth-animating
 * `speaking` when audio is streaming, otherwise `listening` (attentive rest).
 * Both map to the `idle` body pose in `FACE_TO_POSE`, but `speaking` is what
 * drives the viseme mouth on the producer, so the distinction is load-bearing.
 */
function activeSpeakerFace(speaking: boolean): MascotFace {
  return speaking ? 'speaking' : 'listening';
}

/**
 * The face for a single slot in the two-mascot layout.
 *   - greeting / signoff → both wave (`waving` → `hand_wave` pose).
 *   - active: the slot that owns the current audio follows
 *     {@link activeSpeakerFace}; the other slot shows `thinking` — the only
 *     asset-distinct "reacting / listening" body pose (verified against
 *     `FACE_TO_POSE` in riveMaps: `listening` collapses to `idle`, so it would
 *     be visually indistinguishable from the speaker's rest state; `thinking`
 *     is the distinct pose that reads as "the other mascot is paying
 *     attention").
 */
function dualSlotFace(slot: ActiveMascotSlot, input: UseMeetingMascotsInput): MascotFace {
  if (input.phase === 'greeting' || input.phase === 'signoff') return 'waving';
  if (slot === input.activeMascotSlot) return activeSpeakerFace(input.speaking);
  return 'thinking';
}

/**
 * Compute the per-slot render state. Exposed as a standalone pure function so
 * the face table can be exercised directly (the hook is a thin redux wrapper
 * over it).
 */
export function computeMeetingMascotsRenderState(
  dualEnabled: boolean,
  primaryMascotId: string | null,
  secondaryMascotId: string | null,
  input: UseMeetingMascotsInput
): MeetingMascotsRenderState {
  if (!dualEnabled) {
    // Single-mascot path — preserves the original producer behavior exactly:
    // primary follows speaking → speaking/idle, no secondary slot.
    return {
      dualEnabled: false,
      primary: { mascotId: primaryMascotId, face: input.speaking ? 'speaking' : 'idle' },
      secondary: null,
    };
  }
  return {
    dualEnabled: true,
    primary: { mascotId: primaryMascotId, face: dualSlotFace(0, input) },
    secondary: { mascotId: secondaryMascotId, face: dualSlotFace(1, input) },
  };
}

/**
 * Hook form: reads the mascot slice and folds in the live speaking-state +
 * phase to produce the render state the producer composites from.
 */
export function useMeetingMascots(input: UseMeetingMascotsInput): MeetingMascotsRenderState {
  const dualEnabled = useAppSelector(selectDualMascotEnabled);
  const primaryMascotId = useAppSelector(selectSelectedMascotId);
  const secondaryMascotId = useAppSelector(selectSecondaryMascotId);
  // Resolve the voice pair too so the primary slot's mascotId matches exactly
  // what the join payload sent — the pair is the single source of truth and
  // reading it here keeps the on-camera mascot and the spoken voice aligned.
  const pair = useAppSelector(selectMeetingMascotVoicePair);

  const state = computeMeetingMascotsRenderState(
    dualEnabled,
    pair.primary.mascotId ?? primaryMascotId,
    pair.secondary?.mascotId ?? secondaryMascotId,
    input
  );

  log(
    'render state dual=%s phase=%s speaking=%s activeSlot=%d primaryFace=%s secondaryFace=%s',
    state.dualEnabled,
    input.phase,
    input.speaking,
    input.activeMascotSlot,
    state.primary.face,
    state.secondary?.face ?? 'none'
  );

  return state;
}
