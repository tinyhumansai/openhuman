import {
  Fit,
  Layout,
  useRive,
  useViewModel,
  useViewModelInstance,
  useViewModelInstanceColor,
  useViewModelInstanceString,
} from '@rive-app/react-webgl2';
import { type FC, useEffect } from 'react';

import type { MascotFace } from './Ghosty';

export interface RiveMascotProps {
  face?: MascotFace;
  size?: number | string;
  primaryColor?: number;
  secondaryColor?: number;
  /** Raw Oculus 15-set viseme code (e.g. 'sil', 'PP', 'aa') sent directly to
   *  the Rive state machine's `mouthVisemeCode` input. When omitted, defaults
   *  to 'sil' (mouth closed). */
  visemeCode?: string;
}

/**
 * Maps every MascotFace to the closest Rive pose animation. The Rive asset
 * supports: idle, thinking, celebration, bookreading, coffeedrink, writing,
 * bobbateadrink, recording, hand_wave, dancing.
 */
const FACE_TO_POSE: Record<MascotFace, string> = {
  idle: 'idle',
  normal: 'idle',
  sleep: 'idle',
  listening: 'idle',
  thinking: 'thinking',
  confused: 'thinking',
  speaking: 'idle',
  happy: 'idle',
  concerned: 'thinking',
  curious: 'bookreading',
  proud: 'celebration',
  cautious: 'thinking',
  celebrating: 'celebration',
  writing: 'writing',
  reading: 'bookreading',
  recording: 'recording',
  waving: 'hand_wave',
  dancing: 'dancing',
  drinking_coffee: 'coffeedrink',
  drinking_boba: 'bobbateadrink',
};

/**
 * ElevenLabs / Oculus 15-set → Rive asset's `visme_codes` vocabulary.
 * The Rive file uses `ih`/`oh`/`ou` for vowels instead of `E`/`O`/`U`.
 * Codes already in the Rive vocabulary pass through unchanged.
 */
const OCULUS_TO_RIVE_VISEME: Record<string, string> = {
  E: 'ih',
  I: 'ih',
  O: 'oh',
  U: 'ou',
  e: 'ih',
  i: 'ih',
  o: 'oh',
  u: 'ou',
};

function toRiveVisemeCode(oculusCode: string): string {
  return OCULUS_TO_RIVE_VISEME[oculusCode] ?? oculusCode;
}

const RIVE_LAYOUT = new Layout({ fit: Fit.Contain });

export const RiveMascot: FC<RiveMascotProps> = ({
  face = 'idle',
  size = '100%',
  primaryColor,
  secondaryColor,
  visemeCode = 'sil',
}) => {
  const { rive, RiveComponent } = useRive({
    src: '/tiny_mascot.riv',
    stateMachines: 'Main State Machine',
    autoplay: true,
    layout: RIVE_LAYOUT,
  });

  const viewModel = useViewModel(rive, { useDefault: true });
  const vmInstance = useViewModelInstance(viewModel, { useDefault: true, rive });
  const { setValue: setPose } = useViewModelInstanceString('pose', vmInstance);
  const { setValue: setMouthVisemeCode } = useViewModelInstanceString(
    'mouthVisemeCode',
    vmInstance
  );
  const { setValue: setPrimaryColor } = useViewModelInstanceColor('primaryColor', vmInstance);
  const { setValue: setSecondaryColor } = useViewModelInstanceColor('secondaryColor', vmInstance);

  useEffect(() => {
    setPose(FACE_TO_POSE[face] ?? 'idle');
  }, [face, setPose]);

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
