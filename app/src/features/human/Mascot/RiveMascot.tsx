import {
  Fit,
  Layout,
  useRive,
  useViewModel,
  useViewModelInstance,
  useViewModelInstanceColor,
  useViewModelInstanceEnum,
} from '@rive-app/react-webgl2';
import debug from 'debug';
import { type FC, useEffect, useRef } from 'react';

import type { MascotFace } from './Ghosty';
import {
  faceToPose,
  MASCOT_STATE_MACHINE,
  pickAmbientPose,
  type RivePose,
  toRiveVisemeCode,
} from './riveMaps';

const riveLog = debug('human:mascot:rive');

/** Idle dwell before the mascot drifts into an ambient pose (ms). Randomised
 *  in `[MIN, MAX]` so the cadence never feels metronomic. */
const AMBIENT_IDLE_MIN_MS = 6_000;
const AMBIENT_IDLE_MAX_MS = 12_000;
/** How long an ambient pose is held before returning to idle (ms). */
const AMBIENT_HOLD_MIN_MS = 2_500;
const AMBIENT_HOLD_MAX_MS = 5_000;

function randBetween(min: number, max: number): number {
  return min + Math.random() * (max - min);
}

/** Bundled default mascot, served by Vite from `public/`. */
export const DEFAULT_MASCOT_SRC = '/tiny_mascot.riv';

export interface RiveMascotProps {
  face?: MascotFace;
  size?: number | string;
  primaryColor?: number;
  secondaryColor?: number;
  /** Raw Oculus 15-set viseme code (e.g. 'sil', 'PP', 'aa') sent to the Rive
   *  state machine's `mouthVisemeCode` input after normalisation. Defaults to
   *  'sil' (mouth closed). */
  visemeCode?: string;
  /** When true and the mascot is otherwise idle, it drifts through random
   *  ambient poses (thinking, sipping coffee, dancing, …) to feel alive.
   *  Off by default so small previews / frame producers stay still. */
  idlePoseRotation?: boolean;
  /** Override the bundled mascot with a URL to a different `.riv` file.
   *  Ignored when `buffer` is supplied. */
  src?: string;
  /** Render a `.riv` already loaded into memory (e.g. a version-cached custom
   *  mascot from the backend). Takes precedence over `src`. */
  buffer?: ArrayBuffer;
  /** State-machine name inside the `.riv`. Custom mascots are authored against
   *  the same `MascotSM` convention by default. */
  stateMachine?: string;
}

const RIVE_LAYOUT = new Layout({ fit: Fit.Contain });

export const RiveMascot: FC<RiveMascotProps> = ({
  face = 'idle',
  size = '100%',
  primaryColor,
  secondaryColor,
  visemeCode = 'sil',
  idlePoseRotation = false,
  src = DEFAULT_MASCOT_SRC,
  buffer,
  stateMachine = MASCOT_STATE_MACHINE,
}) => {
  // `buffer` (an in-memory custom mascot) wins over `src` (a URL). Passing only
  // one of the two to useRive keeps the runtime from racing both loaders.
  const riveSource = buffer ? { buffer } : { src };
  const { rive, RiveComponent } = useRive({
    ...riveSource,
    stateMachines: stateMachine,
    autoplay: true,
    layout: RIVE_LAYOUT,
  });

  const viewModel = useViewModel(rive, { useDefault: true });
  const vmInstance = useViewModelInstance(viewModel, { useDefault: true, rive });
  const { setValue: setPose } = useViewModelInstanceEnum('pose', vmInstance);
  const { setValue: setMouthVisemeCode } = useViewModelInstanceEnum('mouthVisemeCode', vmInstance);
  const { setValue: setPrimaryColor } = useViewModelInstanceColor('primaryColor', vmInstance);
  const { setValue: setSecondaryColor } = useViewModelInstanceColor('secondaryColor', vmInstance);

  const basePose = faceToPose(face);

  // The driven (face-derived) pose. A real activity pose always shows; `idle`
  // is the resting state the ambient scheduler is free to override below.
  useEffect(() => {
    setPose(basePose);
  }, [basePose, setPose]);

  // Idle pose rotation: a self-rescheduling timer that nudges the mascot into a
  // random ambient pose, holds it, returns to idle, and repeats. Only runs
  // while enabled AND the driven pose is idle; any real activity tears it down
  // (the cleanup restores idle, then the effect above sets the activity pose).
  //
  // We drive Rive imperatively through a ref to the setter so the timer chain
  // survives `setValue` identity changes without resetting its cadence.
  const setPoseRef = useRef(setPose);
  setPoseRef.current = setPose;
  useEffect(() => {
    if (!idlePoseRotation || basePose !== 'idle') return;
    let timer: number | undefined;
    let current: RivePose = 'idle';
    const toIdle = () => {
      current = 'idle';
      setPoseRef.current('idle');
      timer = window.setTimeout(toAmbient, randBetween(AMBIENT_IDLE_MIN_MS, AMBIENT_IDLE_MAX_MS));
    };
    const toAmbient = () => {
      current = pickAmbientPose(current === 'idle' ? undefined : current);
      riveLog('idle pose rotation → %s', current);
      setPoseRef.current(current);
      timer = window.setTimeout(toIdle, randBetween(AMBIENT_HOLD_MIN_MS, AMBIENT_HOLD_MAX_MS));
    };
    timer = window.setTimeout(toAmbient, randBetween(AMBIENT_IDLE_MIN_MS, AMBIENT_IDLE_MAX_MS));
    return () => {
      if (timer !== undefined) window.clearTimeout(timer);
      setPoseRef.current('idle');
    };
  }, [idlePoseRotation, basePose]);

  useEffect(() => {
    setMouthVisemeCode(toRiveVisemeCode(visemeCode));
  }, [visemeCode, setMouthVisemeCode]);

  useEffect(() => {
    if (primaryColor !== undefined) setPrimaryColor(primaryColor);
  }, [primaryColor, setPrimaryColor]);

  useEffect(() => {
    if (secondaryColor !== undefined) setSecondaryColor(secondaryColor);
  }, [secondaryColor, setSecondaryColor]);

  return (
    <div
      style={{
        width: typeof size === 'number' ? `${size}px` : size,
        height: typeof size === 'number' ? `${size}px` : size,
      }}
      data-face={face}>
      <RiveComponent />
    </div>
  );
};
