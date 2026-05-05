import { Player, type PlayerRef } from '@remotion/player';
import { type ComponentType, type FC, useEffect, useMemo, useRef } from 'react';

import type { MascotFace } from './Ghosty';
import type { MascotProps as YellowMascotInnerProps } from './yellow/MascotCharacter';
import { YellowMascotIdle } from './yellow/MascotIdle';
import { YellowMascotTalking } from './yellow/MascotTalking';
import { YellowMascotThinking } from './yellow/MascotThinking';

export interface YellowMascotProps {
  /** High-level state from the agent/voice lifecycle. Mapped to a composition. */
  face?: MascotFace;
  /** Whether to show the wave arm. Only meaningful in idle/listening states. */
  arm?: 'wave' | 'none';
  /** Override SVG element size; defaults to filling the parent. */
  size?: number | string;
  /** Center opacity of the ground shadow gradient. Defaults to 0.35; bump
   *  up when the mascot is rendered very small (floating mascot window) so
   *  the shadow stays readable. */
  groundShadowOpacity?: number;
  /** Replace the warm yellow/amber arm inner-shadow tints with darker
   *  neutrals — for very small renders where the warm tint reads as a
   *  bright halo instead of a shadow. */
  compactArmShading?: boolean;
}

const FPS = 30;
// Composition canvas matches the Remotion side (1080x1080); Player scales it.
const CANVAS = 1080;
// Loop length per state. The Thinking variant we authored loops cleanly at 6s.
const DURATION_FRAMES = FPS * 6;

interface Variant {
  component: ComponentType<YellowMascotInnerProps>;
  inputProps: YellowMascotInnerProps;
}

function variantForFace(face: MascotFace, arm: 'wave' | 'none'): Variant {
  const base: Pick<
    YellowMascotInnerProps,
    'face' | 'recordingColor' | 'loadingColor' | 'greeting' | 'sleeping'
  > = {
    face: 'normal',
    recordingColor: '#ff3b30',
    loadingColor: '#ffffff',
    greeting: false,
    sleeping: false,
  };
  switch (face) {
    case 'thinking':
    case 'confused':
      return {
        component: YellowMascotThinking,
        inputProps: { ...base, arm: 'steady', talking: false, thinking: true },
      };
    case 'speaking':
    case 'happy':
      return {
        component: YellowMascotTalking,
        inputProps: { ...base, arm: 'steady', talking: true, thinking: false },
      };
    case 'listening':
    case 'idle':
    case 'normal':
    case 'concerned':
    default:
      return {
        component: YellowMascotIdle,
        inputProps: { ...base, arm, talking: false, thinking: false },
      };
  }
}

export const YellowMascot: FC<YellowMascotProps> = ({
  face = 'idle',
  arm = 'none',
  size = '100%',
  groundShadowOpacity,
  compactArmShading,
}) => {
  const { component, inputProps } = useMemo(() => {
    const variant = variantForFace(face, arm);
    return {
      component: variant.component,
      inputProps: {
        ...variant.inputProps,
        ...(groundShadowOpacity !== undefined && { groundShadowOpacity }),
        ...(compactArmShading !== undefined && { compactArmShading }),
      },
    };
  }, [face, arm, groundShadowOpacity, compactArmShading]);
  const playerRef = useRef<PlayerRef>(null);

  // Player's `autoPlay` prop is unreliable across browsers / strict-mode mounts
  // (autoplay policy gating, ref attaching after first paint). Kick playback
  // explicitly once the ref is attached and again whenever the variant changes.
  useEffect(() => {
    const p = playerRef.current;
    if (!p) return;
    p.play();
  }, [component]);

  return (
    <div
      style={{
        width: typeof size === 'number' ? `${size}px` : size,
        aspectRatio: '1 / 1',
        // Player draws a black background by default; transparent so the page bg shows through.
        background: 'transparent',
      }}
      data-face={face}>
      <Player
        ref={playerRef}
        component={component as ComponentType<Record<string, unknown>>}
        inputProps={inputProps as unknown as Record<string, unknown>}
        durationInFrames={DURATION_FRAMES}
        fps={FPS}
        compositionWidth={CANVAS}
        compositionHeight={CANVAS}
        loop
        autoPlay
        controls={false}
        clickToPlay={false}
        doubleClickToFullscreen={false}
        style={{ width: '100%', height: '100%', background: 'transparent' }}
      />
    </div>
  );
};
