import type { MascotFace } from './Ghosty';
import type { MascotColor } from './mascotPalette';

export type YellowMascotAssetProfile = 'default' | 'compact';
export type YellowMascotAssetVariant =
  | 'yellow-MascotIdle'
  | 'yellow-MascotTalking'
  | 'yellow-MascotThinking';

export interface YellowMascotAssetRequest {
  arm: 'wave' | 'none';
  compactArmShading?: boolean;
  face: MascotFace;
  groundShadowOpacity?: number;
  mascotColor?: MascotColor;
}

export interface YellowMascotAssetSelection {
  profile: YellowMascotAssetProfile;
  relativePath: string;
  variant: YellowMascotAssetVariant;
}

function resolveVariant(face: MascotFace): YellowMascotAssetVariant {
  switch (face) {
    case 'thinking':
    case 'confused':
      return 'yellow-MascotThinking';
    case 'speaking':
    case 'happy':
      return 'yellow-MascotTalking';
    case 'listening':
    case 'idle':
    case 'normal':
    case 'concerned':
    default:
      return 'yellow-MascotIdle';
  }
}

function resolveProfile({
  compactArmShading,
  groundShadowOpacity,
}: Pick<YellowMascotAssetRequest, 'compactArmShading' | 'groundShadowOpacity'>): YellowMascotAssetProfile | null {
  if (groundShadowOpacity === undefined && compactArmShading === undefined) {
    return 'default';
  }
  if (groundShadowOpacity === 0.75 && compactArmShading === true) {
    return 'compact';
  }
  return null;
}

export function selectYellowMascotAsset(
  request: YellowMascotAssetRequest
): YellowMascotAssetSelection | null {
  if (request.arm !== 'none') {
    return null;
  }

  const profile = resolveProfile(request);
  if (!profile) {
    return null;
  }

  const variant = resolveVariant(request.face);
  const mascotColor = request.mascotColor ?? 'yellow';
  return {
    profile,
    variant,
    relativePath: `generated/remotion/${profile}/${mascotColor}/${variant}.webp`,
  };
}
