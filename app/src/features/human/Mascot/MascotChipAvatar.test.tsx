import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { MascotChipAvatar } from './MascotChipAvatar';
import { getMascotPalette, shadeHex } from './mascotPalette';

describe('MascotChipAvatar', () => {
  it('renders the custom GIF (decorative) when a gifUrl is provided', () => {
    render(<MascotChipAvatar color="yellow" gifUrl="https://example.com/avatar.gif" />);

    const wrapper = screen.getByTestId('mascot-chip-avatar');
    expect(wrapper).toHaveAttribute('data-variant', 'gif');

    const img = wrapper.querySelector('img') as HTMLImageElement;
    expect(img).toBeTruthy();
    expect(img).toHaveAttribute('src', 'https://example.com/avatar.gif');
    // Decorative: the chip's <button> already carries the accessible label.
    expect(img).toHaveAttribute('aria-hidden', 'true');
  });

  it('falls back to the lightweight Ghosty SVG when no gifUrl is set', () => {
    render(<MascotChipAvatar color="yellow" />);

    const wrapper = screen.getByTestId('mascot-chip-avatar');
    expect(wrapper).toHaveAttribute('data-variant', 'ghosty');
    // The static (non-RAF) mascot is an inline SVG, not the heavy WebGL canvas.
    expect(wrapper.querySelector('svg')).toBeTruthy();
    expect(wrapper.querySelector('canvas')).toBeNull();
  });

  it('tints the SVG body with the custom primary colour when color is "custom"', () => {
    render(<MascotChipAvatar color="custom" customPrimary="#abcdef" />);

    // GhostyDefs paints the body fill from bodyColor; the custom hex must appear
    // somewhere in the rendered SVG markup.
    const wrapper = screen.getByTestId('mascot-chip-avatar');
    expect(wrapper.innerHTML.toLowerCase()).toContain('#abcdef');
  });

  it('uses the palette body colour for a named theme', () => {
    const { bodyFill } = getMascotPalette('navy');
    render(<MascotChipAvatar color="navy" />);

    const wrapper = screen.getByTestId('mascot-chip-avatar');
    expect(wrapper.innerHTML.toLowerCase()).toContain(bodyFill.toLowerCase());
  });

  it('renders the bright "flat" body so the chip matches the Rive mascot stage', () => {
    // The flat variant derives a lightened highlight stop from the body colour;
    // its presence proves we are NOT using the dark, moody body gradient.
    const highlight = shadeHex(getMascotPalette('yellow').bodyFill, 0.32);
    render(<MascotChipAvatar color="yellow" />);

    const wrapper = screen.getByTestId('mascot-chip-avatar');
    expect(wrapper.innerHTML.toLowerCase()).toContain(highlight.toLowerCase());
    // The dark-gradient sentinel stop (#050506) must be absent in flat mode.
    expect(wrapper.innerHTML.toLowerCase()).not.toContain('#050506');
  });

  it('applies the requested pixel size to the avatar box', () => {
    render(<MascotChipAvatar color="yellow" size={30} />);

    const wrapper = screen.getByTestId('mascot-chip-avatar');
    expect(wrapper).toHaveStyle({ width: '30px', height: '30px' });
  });
});
