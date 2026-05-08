import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { YellowMascot } from './YellowMascot';

describe('<YellowMascot />', () => {
  it('renders an svg by default with the configured face data attribute', () => {
    const { container } = render(<YellowMascot />);
    const host = container.querySelector('[data-face]') as HTMLElement;
    expect(host).not.toBeNull();
    expect(host.getAttribute('data-face')).toBe('idle');
    expect(container.querySelector('svg')).not.toBeNull();
  });

  it.each([
    ['idle', 'idle'],
    ['speaking', 'speaking'],
    ['thinking', 'thinking'],
    ['confused', 'confused'],
  ] as const)('passes %s through to data-face', (face, expected) => {
    const { container } = render(<YellowMascot face={face} />);
    const host = container.querySelector('[data-face]') as HTMLElement;
    expect(host.getAttribute('data-face')).toBe(expected);
  });

  it('forwards a numeric size prop as a pixel width', () => {
    const { container } = render(<YellowMascot size={48} />);
    const host = container.querySelector('[data-face]') as HTMLElement;
    expect(host.style.width).toBe('48px');
  });

  it('uses the requested mascot color palette in the rendered svg fills', () => {
    const { container: yellow } = render(<YellowMascot />);
    const { container: navy } = render(<YellowMascot mascotColor="navy" />);
    const yellowFill = yellow.querySelector('path[fill]');
    const navyFill = navy.querySelector('path[fill]');
    expect(yellowFill).not.toBeNull();
    expect(navyFill).not.toBeNull();
    expect(yellowFill?.getAttribute('fill')).not.toBe(navyFill?.getAttribute('fill'));
  });
});
