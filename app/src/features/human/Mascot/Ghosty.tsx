import React from 'react';

import { GhostyDefs } from './Defs';
import { ARM_PATH, BODY_PATH, LEFT_LEG_PATH, RIGHT_LEG_PATH, VIEWBOX } from './paths';
import { useMascotClock } from './useMascotClock';
import { type VisemeShape, VISEMES, visemePath } from './visemes';

export type MascotFace = 'normal' | 'listening' | 'thinking' | 'speaking';

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

export const Ghosty: React.FC<GhostyProps> = ({
  bodyColor = '#2a3a55',
  blushColor = '#f4a3a3',
  arm = 'none',
  face = 'normal',
  viseme,
  size = '100%',
  idPrefix = 'mascot',
}) => {
  const t = useMascotClock();

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

  // Blink ~0.2s every 2.6s, offset so frame 0 is eyes open.
  const blinkMs = 2600;
  const blinkOffset = blinkMs / 2;
  const tMs = t * 1000;
  const inBlink = (tMs + blinkOffset) % blinkMs < 200;
  const blinkScale = inBlink ? 0.12 : 1;

  const id = (k: string) => `${idPrefix}-${k}`;
  const bodyFill = `url(#${id('body')})`;
  const dotFill = `url(#${id('dot')})`;

  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${VIEWBOX} ${VIEWBOX}`}
      style={{ overflow: 'visible', display: 'block' }}>
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

        <ellipse cx={360} cy={545} rx={48} ry={22} fill={blushColor} opacity={0.85} />
        <ellipse cx={680} cy={545} rx={48} ry={22} fill={blushColor} opacity={0.85} />

        <g>
          <ellipse cx={415} cy={515} rx={30} ry={40 * blinkScale} fill="#0a0a0a" />
          <ellipse cx={625} cy={515} rx={30} ry={40 * blinkScale} fill="#0a0a0a" />
          {!inBlink && (
            <>
              <circle cx={425} cy={501} r={7} fill="#ffffff" />
              <circle cx={635} cy={501} r={7} fill="#ffffff" />
            </>
          )}
        </g>

        <path d={visemePath(viseme ?? VISEMES.REST)} fill="#0a0a0a" data-face={face} />

        <ellipse cx={360} cy={545} rx={56} ry={26} fill={blushColor} opacity={0.18} />
        <ellipse cx={680} cy={545} rx={56} ry={26} fill={blushColor} opacity={0.18} />
      </g>
    </svg>
  );
};
