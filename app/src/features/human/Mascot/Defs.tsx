import React from 'react';

import { shadeHex } from './mascotPalette';
import { BODY_PATH } from './paths';

/**
 * `shaded` (default) is the original moody, dark-stopped body gradient used by
 * the full-size mascot stage. `flat` is a bright, body-colour-dominant gradient
 * that mirrors the Rive mascot's look — used for compact avatars (e.g. the
 * "Talk to Tiny" chip) so the small mascot matches the big one in the panel
 * rather than reading as a dark blob.
 */
export type GhostyVariant = 'shaded' | 'flat';

export interface GhostyDefsProps {
  idPrefix: string;
  bodyColor: string;
  variant?: GhostyVariant;
}

export const GhostyDefs: React.FC<GhostyDefsProps> = ({
  idPrefix,
  bodyColor,
  variant = 'shaded',
}) => {
  const id = (k: string) => `${idPrefix}-${k}`;
  const flat = variant === 'flat';
  // Derive a soft highlight + edge shadow from the body colour so any palette
  // (including custom hexes) gets a clean, bright 3D body.
  const highlight = shadeHex(bodyColor, 0.32);
  const edge = shadeHex(bodyColor, -0.24);
  return (
    <defs>
      {flat ? (
        <radialGradient id={id('body')} cx="0.35" cy="0.3" r="0.95">
          <stop offset="0%" stopColor={highlight} />
          <stop offset="45%" stopColor={bodyColor} />
          <stop offset="100%" stopColor={edge} />
        </radialGradient>
      ) : (
        <radialGradient id={id('body')} cx="0.32" cy="0.28" r="1.05">
          <stop offset="0%" stopColor="#45454a" />
          <stop offset="15%" stopColor="#393940" />
          <stop offset="30%" stopColor="#2d2d33" />
          <stop offset="45%" stopColor={bodyColor} />
          <stop offset="60%" stopColor="#1a1a1e" />
          <stop offset="75%" stopColor="#121215" />
          <stop offset="88%" stopColor="#0a0a0c" />
          <stop offset="100%" stopColor="#050506" />
        </radialGradient>
      )}
      {flat ? (
        <radialGradient id={id('dot')} cx="0.35" cy="0.3" r="1">
          <stop offset="0%" stopColor={highlight} />
          <stop offset="55%" stopColor={bodyColor} />
          <stop offset="100%" stopColor={edge} />
        </radialGradient>
      ) : (
        <radialGradient id={id('dot')} cx="0.35" cy="0.3" r="1">
          <stop offset="0%" stopColor="#45454a" />
          <stop offset="20%" stopColor="#363639" />
          <stop offset="45%" stopColor={bodyColor} />
          <stop offset="70%" stopColor="#15151a" />
          <stop offset="100%" stopColor="#050507" />
        </radialGradient>
      )}
      <filter id={id('grain')} x="0%" y="0%" width="100%" height="100%">
        <feTurbulence
          type="fractalNoise"
          baseFrequency="0.9"
          numOctaves="2"
          stitchTiles="stitch"
          result="noise"
        />
        <feColorMatrix
          in="noise"
          type="matrix"
          values="0 0 0 0 1  0 0 0 0 1  0 0 0 0 1  0 0 0 0.06 0"
        />
      </filter>
      <filter id={id('soft')} x="-30%" y="-30%" width="160%" height="160%">
        <feGaussianBlur stdDeviation="30" />
      </filter>
      <filter id={id('drop')} x="-20%" y="-20%" width="140%" height="160%">
        <feGaussianBlur in="SourceAlpha" stdDeviation="14" />
        <feOffset dx="0" dy="22" result="off" />
        <feComponentTransfer>
          <feFuncA type="linear" slope="0.45" />
        </feComponentTransfer>
        <feMerge>
          <feMergeNode />
          <feMergeNode in="SourceGraphic" />
        </feMerge>
      </filter>
      <radialGradient id={id('ground')} cx="0.5" cy="0.5" r="0.5">
        <stop offset="0%" stopColor="#000000" stopOpacity="0.35" />
        <stop offset="100%" stopColor="#000000" stopOpacity="0" />
      </radialGradient>
      <clipPath id={id('body-clip')}>
        <path d={BODY_PATH} />
      </clipPath>
    </defs>
  );
};
