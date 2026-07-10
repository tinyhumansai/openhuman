export { Ghosty } from './Ghosty';
export type { GhostyProps, MascotFace } from './Ghosty';
export { CustomGifMascot } from './CustomGifMascot';
export type { CustomGifMascotProps } from './CustomGifMascot';
export { MascotChipAvatar } from './MascotChipAvatar';
export type { MascotChipAvatarProps } from './MascotChipAvatar';
export { RiveMascot, DEFAULT_MASCOT_SRC } from './RiveMascot';
export type { RiveMascotProps } from './RiveMascot';
export { ManifestRiveMascot } from './ManifestRiveMascot';
export type { ManifestRiveMascotProps } from './ManifestRiveMascot';
export {
  fetchMascotManifest,
  defaultMascot,
  findMascot,
  loadManifestRiv,
} from './manifest/manifestService';
export type {
  MascotManifest,
  MascotManifestEntry,
  MascotStateEngine,
  MascotManifestChannel,
} from './manifest/types';
export { BackendRiveMascot } from './backend/BackendRiveMascot';
export type { BackendRiveMascotProps } from './backend/BackendRiveMascot';
export { lerpViseme, VISEMES, visemePath } from './visemes';
export type { VisemeId, VisemeShape } from './visemes';
export { getMascotPalette, hexToArgbInt } from './mascotPalette';
export type { MascotColor, MascotPalette } from './mascotPalette';
