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
 * - `curious` — attentive/interested; user asked something engaging or agent
 *   is exploring an interesting problem.
 * - `proud` — task fully completed after meaningful tool/subagent work.
 * - `cautious` — gentle warning; less severe than `concerned`.
 *
 * Activity poses — driven by what the agent is actively doing:
 * - `celebrating` — success animation after meaningful work.
 * - `writing` — agent is editing/creating files.
 * - `reading` — agent is browsing or reading content.
 * - `recording` — agent is capturing screen or recording.
 * - `waving` — greeting or hello gesture.
 * - `dancing` — celebratory/playful animation.
 * - `drinking_coffee` — agent is processing / long-running task.
 * - `drinking_boba` — relaxed variant of processing.
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
  | 'curious'
  | 'proud'
  | 'cautious'
  | 'celebrating'
  | 'writing'
  | 'reading'
  | 'recording'
  | 'waving'
  | 'dancing'
  | 'drinking_coffee'
  | 'drinking_boba'
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
  // Wide, attentive eyes with raised brows — the mascot is engaged and
  // interested in what is happening.
  curious: {
    eyeScaleY: 1.1,
    eyeScaleX: 1.05,
    browTilt: -10,
    browDy: -8,
    showBrows: true,
    blushOpacity: 0.8,
  },
  // Squinted-happy with full blush — task completed after real work.
  // Visually distinct from `happy` (less squint, brows soft) so it reads
  // as satisfaction rather than a quick acknowledgement.
  proud: {
    eyeScaleY: 0.55,
    eyeScaleX: 1.15,
    browTilt: -4,
    browDy: -4,
    showBrows: false,
    blushOpacity: 1,
  },
  // Gentle worry — a heads-up rather than a failure. Softer than `concerned`
  // (less brow tilt, lighter blush reduction).
  cautious: {
    eyeScaleY: 0.9,
    eyeScaleX: 0.95,
    browTilt: 10,
    browDy: -3,
    showBrows: true,
    blushOpacity: 0.65,
  },
  celebrating: {
    eyeScaleY: 0.4,
    eyeScaleX: 1.15,
    browTilt: -8,
    browDy: -8,
    showBrows: false,
    blushOpacity: 1,
  },
  writing: {
    eyeScaleY: 0.75,
    eyeScaleX: 1,
    browTilt: -2,
    browDy: -1,
    showBrows: false,
    blushOpacity: 0.7,
  },
  reading: {
    eyeScaleY: 0.85,
    eyeScaleX: 1.05,
    browTilt: -6,
    browDy: -4,
    showBrows: true,
    blushOpacity: 0.75,
  },
  recording: {
    eyeScaleY: 1,
    eyeScaleX: 1,
    browTilt: 0,
    browDy: 0,
    showBrows: false,
    blushOpacity: 0.9,
  },
  waving: {
    eyeScaleY: 0.5,
    eyeScaleX: 1.1,
    browTilt: -6,
    browDy: -6,
    showBrows: false,
    blushOpacity: 1,
  },
  dancing: {
    eyeScaleY: 0.4,
    eyeScaleX: 1.15,
    browTilt: -8,
    browDy: -8,
    showBrows: false,
    blushOpacity: 1,
  },
  drinking_coffee: {
    eyeScaleY: 0.6,
    eyeScaleX: 1,
    browTilt: 0,
    browDy: 0,
    showBrows: false,
    blushOpacity: 0.8,
  },
  drinking_boba: {
    eyeScaleY: 0.55,
    eyeScaleX: 1.05,
    browTilt: 0,
    browDy: 0,
    showBrows: false,
    blushOpacity: 0.85,
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
    case 'curious':
      // Small open oval — slight "oh?" shape.
      return 'M500,578 Q520,598 540,578 Q520,610 500,578 Z';
    case 'proud':
      // Relaxed upward curve, wider than happy but not a full grin.
      return 'M468,570 Q520,625 572,570 Q520,600 468,570 Z';
    case 'cautious':
      // Slight downward turn — concern lite.
      return 'M482,600 Q520,568 558,600 Q520,585 482,600 Z';
    case 'celebrating':
    case 'dancing':
      return 'M460,565 Q520,635 580,565 Q520,605 460,565 Z';
    case 'waving':
      return 'M460,565 Q520,635 580,565 Q520,605 460,565 Z';
    case 'writing':
    case 'reading':
      return 'M488,585 Q520,595 552,585 Q520,592 488,585 Z';
    case 'recording':
      return 'M495,580 Q520,600 545,580 Q520,615 495,580 Z';
    case 'drinking_coffee':
    case 'drinking_boba':
      return 'M500,578 Q520,598 540,578 Q520,610 500,578 Z';
    default:
      return visemePath(VISEMES.REST);
  }
}
