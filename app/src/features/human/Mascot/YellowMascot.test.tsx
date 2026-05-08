import { fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { __resetMascotManifestForTests, setAvailableMascotColors } from './mascotManifest';
import { YellowMascot } from './YellowMascot';

describe('<YellowMascot />', () => {
  beforeEach(() => {
    __resetMascotManifestForTests();
  });

  afterEach(() => {
    __resetMascotManifestForTests();
  });

  it('renders the pre-rendered yellow asset for the default idle state', () => {
    render(<YellowMascot />);
    const img = document.querySelector('img') as HTMLImageElement;
    expect(img).not.toBeNull();
    expect(img.src).toContain('generated/remotion/default/yellow/yellow-MascotIdle.webp');
  });

  it('honors the mascotColor prop when the manifest reports it available', () => {
    setAvailableMascotColors(['yellow', 'navy']);
    render(<YellowMascot face="speaking" mascotColor="navy" />);
    const img = document.querySelector('img') as HTMLImageElement;
    expect(img).not.toBeNull();
    expect(img.src).toContain('generated/remotion/default/navy/yellow-MascotTalking.webp');
  });

  it('falls back to yellow when the requested color is unavailable', () => {
    render(<YellowMascot mascotColor="green" />);
    const img = document.querySelector('img') as HTMLImageElement;
    expect(img).not.toBeNull();
    expect(img.src).toContain('/yellow/yellow-MascotIdle.webp');
  });

  it('renders nothing when the prop combination is unsupported', () => {
    const { container } = render(<YellowMascot arm="wave" />);
    expect(container.querySelector('img')).toBeNull();
  });

  it('renders the compact profile asset when compact props are provided', () => {
    render(<YellowMascot face="confused" groundShadowOpacity={0.75} compactArmShading={true} />);
    const img = document.querySelector('img') as HTMLImageElement;
    expect(img).not.toBeNull();
    expect(img.src).toContain('generated/remotion/compact/yellow/yellow-MascotThinking.webp');
  });

  it('removes the asset after an image load error', () => {
    const { container } = render(<YellowMascot />);
    const img = container.querySelector('img');
    expect(img).not.toBeNull();
    fireEvent.error(img!);
    expect(container.querySelector('img')).toBeNull();
  });

  it('accepts a numeric size prop and applies it as a CSS width', () => {
    const { container } = render(<YellowMascot size={48} />);
    const wrapper = container.firstChild as HTMLElement;
    expect(wrapper.style.width).toBe('48px');
  });
});
