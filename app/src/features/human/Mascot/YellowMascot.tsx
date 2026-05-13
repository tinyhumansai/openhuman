import { type ComponentType, type FC, useMemo } from 'react';

import type { MascotFace } from './Ghosty';
import type { MascotColor } from './mascotPalette';
import { FrameProvider } from './yellow/frameContext';
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
  /** Center opacity of the ground shadow gradient — pass through to MascotCharacter. */
  groundShadowOpacity?: number;
  /** Use the compact arm shading variant — pass through to MascotCharacter. */
  compactArmShading?: boolean;
  /** Mascot color palette. Defaults to yellow. */
  mascotColor?: MascotColor;
}

const FPS = 30;
// Logical canvas size reported via useVideoConfig() to the inner compositions.
// They use width/height for layout math (e.g. transform origins). The actual
// on-screen size comes from the wrapper div + the SVG's CSS width/height.
const CANVAS = 1000;
// Loop length per state. The Thinking variant we authored loops cleanly at 6s.
const DURATION_FRAMES = FPS * 6;

type ExtendedInnerProps = YellowMascotInnerProps & {
  groundShadowOpacity?: number;
  compactArmShading?: boolean;
};

interface Variant {
  component: ComponentType<ExtendedInnerProps>;
  inputProps: ExtendedInnerProps;
}

function variantForFace(
  face: MascotFace,
  arm: 'wave' | 'none',
  extras: Pick<YellowMascotInnerProps, 'mascotColor'>
): Variant {
  const base: Pick<
    YellowMascotInnerProps,
    'face' | 'recordingColor' | 'loadingColor' | 'greeting' | 'sleeping' | 'mascotColor'
  > = {
    face: 'normal',
    recordingColor: '#ff3b30',
    loadingColor: '#ffffff',
    greeting: false,
    sleeping: false,
    mascotColor: extras.mascotColor ?? 'yellow',
  };
  switch (face) {
    case 'sleep':
      return {
        component: YellowMascotIdle,
        inputProps: { ...base, sleeping: true, arm: 'none', talking: false, thinking: false },
      };
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
  mascotColor = 'yellow',
}) => {
  const { Component, inputProps } = useMemo(() => {
    const variant = variantForFace(face, arm, { mascotColor });
    const merged: ExtendedInnerProps = {
      ...variant.inputProps,
      ...(groundShadowOpacity !== undefined ? { groundShadowOpacity } : {}),
      ...(compactArmShading !== undefined ? { compactArmShading } : {}),
    };
    return { Component: variant.component, inputProps: merged };
  }, [face, arm, mascotColor, groundShadowOpacity, compactArmShading]);

  return (
    <div
      className="mascot-yellow-host"
      style={{
        width: typeof size === 'number' ? `${size}px` : size,
        aspectRatio: '1 / 1',
        background: 'transparent',
        position: 'relative',
      }}
      data-face={face}>
      {/* MascotCharacter sets its <svg> to a fixed pixel size derived from
          useVideoConfig().width, then wraps it in an AbsoluteFill that fills
          our parent. With Player gone we override that fixed size via CSS so
          the SVG fills its container — the viewBox handles vector scaling. */}
      <style>{`
        .mascot-yellow-host svg { width: 100% !important; height: 100% !important; }
      `}</style>
      <FrameProvider
        fps={FPS}
        width={CANVAS}
        height={CANVAS}
        durationInFrames={face === 'sleep' ? FPS * 600 : DURATION_FRAMES}>
        <Component {...inputProps} />
      </FrameProvider>
    </div>
  );
};
