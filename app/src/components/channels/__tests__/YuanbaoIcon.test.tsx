import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import YuanbaoIcon from '../YuanbaoIcon';

describe('YuanbaoIcon', () => {
  it('renders an inline SVG with the default size class', () => {
    const { container } = render(<YuanbaoIcon />);
    const svg = container.querySelector('svg');
    expect(svg).not.toBeNull();
    expect(svg).toHaveAttribute('aria-hidden', 'true');
    expect(svg?.getAttribute('class')).toContain('w-5');
    expect(svg?.getAttribute('class')).toContain('h-5');
  });

  it('applies a custom className override', () => {
    const { container } = render(<YuanbaoIcon className="w-10 h-10 text-amber-500" />);
    const svg = container.querySelector('svg');
    expect(svg?.getAttribute('class')).toBe('w-10 h-10 text-amber-500');
  });

  it('generates a unique clipPath id per instance so duplicate icons do not collide', () => {
    const { container } = render(
      <>
        <YuanbaoIcon />
        <YuanbaoIcon />
      </>
    );
    const clips = container.querySelectorAll('clipPath');
    expect(clips.length).toBe(2);
    const id1 = clips[0].getAttribute('id');
    const id2 = clips[1].getAttribute('id');
    expect(id1).toBeTruthy();
    expect(id2).toBeTruthy();
    expect(id1).not.toBe(id2);
  });
});
