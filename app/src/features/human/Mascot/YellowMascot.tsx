import { Player, type PlayerRef } from '@remotion/player';
import { type ComponentType, type FC, useEffect, useMemo, useRef } from 'react';

import type { MascotFace } from './Ghosty';
import type { MascotColor } from './mascotPalette';
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
// Composition canvas. Render at 480×480 — the SVG is vector so the Player
// scales it up to the on-screen size via CSS, and we pay roughly (480/1080)² ≈
// 5× less per-frame filter rasterization (each `feColorMatrix` + inner shadow
// is fundamentally O(canvas pixels)). At typical UI sizes the difference vs
// 1080 is invisible; dropping below ~360 starts to soften antialiased edges.
const CANVAS = 480;
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
  const { component, inputProps } = useMemo(() => {
    const variant = variantForFace(face, arm, { mascotColor });
    const merged: ExtendedInnerProps = {
      ...variant.inputProps,
      ...(groundShadowOpacity !== undefined ? { groundShadowOpacity } : {}),
      ...(compactArmShading !== undefined ? { compactArmShading } : {}),
    };
    return { component: variant.component, inputProps: merged };
  }, [face, arm, mascotColor, groundShadowOpacity, compactArmShading]);
  const playerRef = useRef<PlayerRef>(null);

  // Player's `autoPlay` prop races with its internal init: on a cold mount the
  // ref is attached but the internal frame loop hasn't been wired yet, so the
  // single play() call we used to make would silently no-op — the SVG renders
  // its first frame and freezes there until the user interacts (e.g. tab away
  // and back, which remounts when state is hot). Retry play() on rAF until
  // isPlaying() reports true, with a hard stop after ~30 frames to avoid
  // looping forever in failure modes.
  useEffect(() => {
    const p = playerRef.current;
    if (!p) return;
    let raf = 0;
    let attempts = 0;
    const tick = () => {
      const player = playerRef.current;
      if (!player) return;
      if (player.isPlaying()) return;
      player.play();
      attempts += 1;
      if (attempts < 30) raf = window.requestAnimationFrame(tick);
    };
    raf = window.requestAnimationFrame(tick);
    return () => window.cancelAnimationFrame(raf);
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
        // No audio in mascot compositions — skip the shared <audio> tag pool
        // that Player otherwise pre-mounts on first init. Removing those tags
        // is what was actually delaying first-frame playback (browser audio
        // context boot can be hundreds of ms on a cold tab).
        numberOfSharedAudioTags={0}
        style={{ width: '100%', height: '100%', background: 'transparent' }}
      />
    </div>
  );
};
