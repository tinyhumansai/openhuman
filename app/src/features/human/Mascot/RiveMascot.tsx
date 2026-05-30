import {
  Fit,
  Layout,
  useRive,
  useViewModel,
  useViewModelInstance,
  useViewModelInstanceBoolean,
  useViewModelInstanceColor,
  useViewModelInstanceString,
} from '@rive-app/react-webgl2';
import { type FC, useEffect } from 'react';

import type { MascotFace } from './Ghosty';
import type { VisemeId } from './visemes';

export interface RiveMascotProps {
  face?: MascotFace;
  size?: number | string;
  primaryColor?: number;
  secondaryColor?: number;
  viseme?: VisemeId;
}

const SPEAKING_FACES: ReadonlySet<MascotFace> = new Set(['speaking', 'happy']);

const FACE_TO_POSE: Record<MascotFace, string> = {
  idle: 'idle',
  normal: 'idle',
  sleep: 'sleeping',
  listening: 'idle',
  thinking: 'thinking',
  confused: 'thinking',
  speaking: 'idle',
  happy: 'idle',
  concerned: 'idle',
  // New emotional states — map to the nearest available Rive pose until
  // dedicated animations are added to the .riv asset.
  curious: 'thinking',
  proud: 'idle',
  cautious: 'idle',
};

const RIVE_LAYOUT = new Layout({ fit: Fit.Contain });

export const RiveMascot: FC<RiveMascotProps> = ({
  face = 'idle',
  size = '100%',
  primaryColor,
  secondaryColor,
  viseme = 'REST',
}) => {
  const { rive, RiveComponent } = useRive({
    src: '/tiny_mascot.riv',
    stateMachines: 'Main State Machine',
    autoplay: true,
    layout: RIVE_LAYOUT,
  });

  const viewModel = useViewModel(rive, { useDefault: true });
  const vmInstance = useViewModelInstance(viewModel, { useDefault: true, rive });
  const { setValue: setMouthOpen } = useViewModelInstanceBoolean('mouthOpen', vmInstance);
  const { setValue: setPose } = useViewModelInstanceString('pose', vmInstance);
  const { setValue: setViseme } = useViewModelInstanceString('viseme', vmInstance);
  const { setValue: setPrimaryColor } = useViewModelInstanceColor('primaryColor', vmInstance);
  const { setValue: setSecondaryColor } = useViewModelInstanceColor('secondaryColor', vmInstance);

  useEffect(() => {
    setMouthOpen(SPEAKING_FACES.has(face!));
    setPose(FACE_TO_POSE[face!] ?? 'idle');
  }, [face, setMouthOpen, setPose]);

  useEffect(() => {
    setViseme(viseme);
  }, [viseme, setViseme]);

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
