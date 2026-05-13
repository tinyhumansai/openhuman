import React from 'react';

import { GhostyDefs } from './Defs';
import { ARM_PATH, BODY_PATH, LEFT_LEG_PATH, RIGHT_LEG_PATH, VIEWBOX } from './paths';
import { useMascotClock } from './useMascotClock';
import { visemePath, VISEMES, type VisemeShape } from './visemes';

/**
 * Discrete face presets the mascot can wear. The state vocabulary mirrors the
 * agent + voice lifecycle so the renderer stays presentation-only:
 *
 * - `idle` — at rest, no active turn.
 * - `listening` — user is dictating / mic is hot.
 * - `thinking` — first inference call in flight.
 * - `confused` — agent is iterating, calling tools, or otherwise burning rounds.
 * - `speaking` — text or audio is streaming back; the renderer drives the
 *   mouth from `viseme` rather than from `face`.
 * - `happy` — short post-turn acknowledgement before falling back to `idle`.
 * - `concerned` — error / failed tool / unavailable voice path.
 *
 * `normal` is the legacy alias for `idle` and stays accepted for backwards
 * compatibility with older callers.
 */
export type MascotFace =
  | 'sleep'
  | 'idle'
  | 'listening'
  | 'thinking'
  | 'confused'
  | 'speaking'
  | 'happy'
  | 'concerned'
  | 'normal';

export interface GhostyProps {
  bodyColor?: string;
  blushColor?: string;
  arm?: 'wave' | 'none';
  face?: MascotFace;
  /** Active mouth shape. When omitted, the mouth rests in a smile. */
  viseme?: VisemeShape;
  /** Override SVG element size; defaults to filling the parent. */
  size?: number | string;
  idPrefix?: string;
}

interface FacePreset {
  /** Vertical squash of the eyes (1 = round, < 1 = squinted). */
  eyeScaleY: number;
  /** Horizontal scale of the eyes. */
  eyeScaleX: number;
  /** Eyebrow tilt in degrees — positive points the inner brow up (worried). */
  browTilt: number;
  /** Vertical brow offset — negative is higher (raised). */
  browDy: number;
  /** Whether to render eyebrows at all. */
  showBrows: boolean;
  /** Blush intensity multiplier. */
  blushOpacity: number;
}

const FACE_PRESETS: Record<Exclude<MascotFace, 'normal'>, FacePreset> = {
  sleep: {
    eyeScaleY: 0.1,
    eyeScaleX: 1,
    browTilt: 0,
    browDy: 2,
    showBrows: false,
    blushOpacity: 0.5,
  },
  idle: {
    eyeScaleY: 1,
    eyeScaleX: 1,
    browTilt: 0,
    browDy: 0,
    showBrows: false,
    blushOpacity: 0.85,
  },
  listening: {
    eyeScaleY: 1.05,
    eyeScaleX: 1.05,
    browTilt: -8,
    browDy: -10,
    showBrows: true,
    blushOpacity: 0.9,
  },
  thinking: {
    eyeScaleY: 0.7,
    eyeScaleX: 1,
    browTilt: -4,
    browDy: -2,
    showBrows: true,
    blushOpacity: 0.6,
  },
  confused: {
    eyeScaleY: 0.85,
    eyeScaleX: 0.95,
    browTilt: 14,
    browDy: -4,
    showBrows: true,
    blushOpacity: 0.55,
  },
  speaking: {
    eyeScaleY: 1,
    eyeScaleX: 1,
    browTilt: 0,
    browDy: 0,
    showBrows: false,
    blushOpacity: 0.95,
  },
  happy: {
    eyeScaleY: 0.45,
    eyeScaleX: 1.1,
    browTilt: -6,
    browDy: -6,
    showBrows: false,
    blushOpacity: 1,
  },
  concerned: {
    eyeScaleY: 0.95,
    eyeScaleX: 0.95,
    browTilt: 22,
    browDy: -2,
    showBrows: true,
    blushOpacity: 0.5,
  },
};

function presetFor(face: MascotFace): FacePreset {
  return FACE_PRESETS[face === 'normal' ? 'idle' : face];
}

export const Ghosty: React.FC<GhostyProps> = ({
  bodyColor = '#2a3a55',
  blushColor = '#f4a3a3',
  arm = 'none',
  face = 'idle',
  viseme,
  size = '100%',
  idPrefix = 'mascot',
}) => {
  const t = useMascotClock();
  const preset = presetFor(face);

  // Gentle bob for the whole character.
  const bob = Math.sin(t * Math.PI * 1.2) * 14;

  // Top dot drifts independently and squashes when it presses into the body.
  const dotPhase = t * Math.PI * 1.0;
  const dotDx = Math.sin(dotPhase * 0.7) * 6;
  const dotDy = Math.sin(dotPhase) * 9;
  const press = Math.max(0, Math.sin(dotPhase));
  const dotSquashY = 1 - 0.08 * press;
  const dotSquashX = 1 + 0.05 * press;

  const wave = arm === 'wave' ? Math.sin(t * Math.PI * 2.4) * 12 : 0;

  // Blink ~0.2s every 2.6s, offset so frame 0 is eyes open. While `thinking`
  // we slow the blink down a touch so the squint reads as a sustained pose.
  const blinkMs = face === 'thinking' ? 4200 : 2600;
  const blinkOffset = blinkMs / 2;
  const tMs = t * 1000;
  const inBlink = (tMs + blinkOffset) % blinkMs < 200;
  const blinkScale = inBlink ? 0.12 : 1;

  const id = (k: string) => `${idPrefix}-${k}`;
  const bodyFill = `url(#${id('body')})`;
  const dotFill = `url(#${id('dot')})`;

  // Restful mouth path varies by face so a non-speaking expression still reads.
  const restMouth = restMouthPath(face);

  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${VIEWBOX} ${VIEWBOX}`}
      style={{ overflow: 'visible', display: 'block' }}
      data-face={face}>
      <GhostyDefs idPrefix={idPrefix} bodyColor={bodyColor} />

      <g
        transform={`translate(500, 970) scale(${1 - bob / 600}, 1)`}
        style={{ transformOrigin: '500px 970px' }}>
        <ellipse cx={0} cy={0} rx={260} ry={28} fill={`url(#${id('ground')})`} />
      </g>

      <g filter={`url(#${id('drop')})`}>
        <path d={LEFT_LEG_PATH} fill={bodyFill} />
        <path d={RIGHT_LEG_PATH} fill={bodyFill} />
      </g>

      <g transform={`translate(0, ${bob})`} filter={`url(#${id('drop')})`}>
        <g
          transform={
            `translate(${dotDx}, ${dotDy}) ` +
            `translate(520 240) scale(${dotSquashX} ${dotSquashY}) translate(-520 -240)`
          }>
          <ellipse cx={520} cy={155} rx={92} ry={88} fill={dotFill} />
          <ellipse cx={490} cy={120} rx={24} ry={14} fill="#ffffff" opacity={0.18} />
        </g>

        {arm !== 'none' && (
          <g transform={`rotate(${wave} 820 590)`}>
            <path d={ARM_PATH} fill={bodyFill} />
          </g>
        )}

        <path d={BODY_PATH} fill={bodyFill} />

        <g clipPath={`url(#${id('body-clip')})`}>
          <g filter={`url(#${id('soft')})`}>
            <ellipse cx={340} cy={380} rx={220} ry={160} fill="#ffffff" opacity={0.09} />
            <ellipse cx={720} cy={800} rx={280} ry={170} fill="#000000" opacity={0.45} />
          </g>
          <rect x={0} y={0} width={1000} height={1000} filter={`url(#${id('grain')})`} />
        </g>

        <ellipse
          cx={360}
          cy={545}
          rx={48}
          ry={22}
          fill={blushColor}
          opacity={0.85 * preset.blushOpacity}
        />
        <ellipse
          cx={680}
          cy={545}
          rx={48}
          ry={22}
          fill={blushColor}
          opacity={0.85 * preset.blushOpacity}
        />

        {preset.showBrows && (
          <g fill="#0a0a0a" data-face-brows={face}>
            <rect
              x={385}
              y={455 + preset.browDy}
              width={60}
              height={9}
              rx={4}
              transform={`rotate(${-preset.browTilt} 415 ${460 + preset.browDy})`}
            />
            <rect
              x={595}
              y={455 + preset.browDy}
              width={60}
              height={9}
              rx={4}
              transform={`rotate(${preset.browTilt} 625 ${460 + preset.browDy})`}
            />
          </g>
        )}

        <g>
          <ellipse
            cx={415}
            cy={515}
            rx={30 * preset.eyeScaleX}
            ry={40 * preset.eyeScaleY * blinkScale}
            fill="#0a0a0a"
          />
          <ellipse
            cx={625}
            cy={515}
            rx={30 * preset.eyeScaleX}
            ry={40 * preset.eyeScaleY * blinkScale}
            fill="#0a0a0a"
          />
          {!inBlink && (
            <>
              <circle cx={425} cy={501} r={7} fill="#ffffff" />
              <circle cx={635} cy={501} r={7} fill="#ffffff" />
            </>
          )}
        </g>

        {face === 'speaking' ? (
          <path d={visemePath(viseme ?? VISEMES.REST)} fill="#0a0a0a" data-face={face} />
        ) : (
          <path d={restMouth} fill="#0a0a0a" data-face={face} />
        )}

        <ellipse
          cx={360}
          cy={545}
          rx={56}
          ry={26}
          fill={blushColor}
          opacity={0.18 * preset.blushOpacity}
        />
        <ellipse
          cx={680}
          cy={545}
          rx={56}
          ry={26}
          fill={blushColor}
          opacity={0.18 * preset.blushOpacity}
        />
      </g>
    </svg>
  );
};

/**
 * Closed-mouth shape for non-speaking states. Speaking is handled separately
 * via `visemePath` so the mouth tracks the audio.
 */
function restMouthPath(face: MascotFace): string {
  switch (face) {
    case 'sleep':
      // Tiny flat line — sleeping, mouth barely visible.
      return 'M496,588 Q520,593 544,588 Q520,592 496,588 Z';
    case 'happy':
      // Wider grin.
      return 'M460,565 Q520,635 580,565 Q520,605 460,565 Z';
    case 'concerned':
      // Inverted curve — frown.
      return 'M478,605 Q520,560 562,605 Q520,590 478,605 Z';
    case 'confused':
      // Slight side-tilt.
      return 'M478,580 Q520,610 562,575 Q520,597 478,580 Z';
    case 'thinking':
      // Small straight pursed line.
      return 'M488,585 Q520,595 552,585 Q520,592 488,585 Z';
    case 'listening':
      // Open soft "o".
      return 'M495,580 Q520,600 545,580 Q520,615 495,580 Z';
    default:
      return visemePath(VISEMES.REST);
  }
}
