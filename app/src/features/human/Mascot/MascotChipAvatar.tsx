// ---------------------------------------------------------------------------
// MascotChipAvatar
//
// A small, on-brand avatar of the user's mascot for compact, always-on UI
// (e.g. the "Talk to Tiny" face-mode chip). It renders the user's custom GIF
// when one is set, otherwise the lightweight `Ghosty` SVG tinted with their
// chosen palette colour.
//
// It deliberately never uses the heavy WebGL `RiveMascot`: a corner chip should
// not spin up a second GPU animation runtime. The SVG path is rendered with
// `animated={false}` so there is no idle RAF cost — the mascot only comes to
// life via the parent's hover treatment (e.g. `group-hover:animate-wiggle`).
// ---------------------------------------------------------------------------
import { type FC } from 'react';

import { Ghosty } from './Ghosty';
import { getMascotPalette, type MascotColor } from './mascotPalette';

export interface MascotChipAvatarProps {
  /** The user's selected mascot colour theme. */
  color: MascotColor;
  /** Custom primary body colour, used only when `color === 'custom'`. */
  customPrimary?: string | null;
  /** Custom mascot GIF URL — when set, takes precedence over the SVG mascot. */
  gifUrl?: string | null;
  /** Diameter of the avatar in pixels. */
  size?: number;
}

export const MascotChipAvatar: FC<MascotChipAvatarProps> = ({
  color,
  customPrimary,
  gifUrl,
  size = 22,
}) => {
  const palette = getMascotPalette(color);
  const bodyColor = color === 'custom' && customPrimary ? customPrimary : palette.bodyFill;

  if (gifUrl) {
    return (
      <span
        className="inline-flex shrink-0 items-center justify-center overflow-hidden rounded-full"
        style={{ width: size, height: size }}
        data-testid="mascot-chip-avatar"
        data-variant="gif">
        <img src={gifUrl} alt="" aria-hidden="true" className="h-full w-full object-cover" />
      </span>
    );
  }

  return (
    <span
      className="inline-flex shrink-0 items-center justify-center"
      style={{ width: size, height: size }}
      data-testid="mascot-chip-avatar"
      data-variant="ghosty">
      <Ghosty
        bodyColor={bodyColor}
        face="idle"
        arm="none"
        size={size}
        animated={false}
        variant="flat"
      />
    </span>
  );
};

export default MascotChipAvatar;
