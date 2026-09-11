import { render, screen } from '@testing-library/react';
import { createRef } from 'react';
import { describe, expect, it } from 'vitest';

import { AvatarFallback, AvatarImage, AvatarRoot } from './Avatar';

const RAW_PALETTE = /\b(?:bg|text|border|ring)-(?:neutral|stone|slate|canvas|white|black)\b/;

describe('Avatar', () => {
  it('renders the fallback while the image has not loaded', () => {
    render(
      <AvatarRoot data-testid="avatar">
        <AvatarImage src="https://example.test/a.png" alt="Ada" />
        <AvatarFallback>AD</AvatarFallback>
      </AvatarRoot>
    );

    // jsdom never fires `load`, so the fallback is the visible branch — which
    // is exactly the state a broken or slow avatar URL produces in production.
    expect(screen.getByText('AD')).toHaveAttribute('data-slot', 'avatar-fallback');
    expect(screen.queryByRole('img')).toBeNull();
  });

  it('forwards ...rest and data-testid onto the root', () => {
    render(
      <AvatarRoot data-testid="avatar" id="user-avatar" aria-label="Ada Lovelace">
        <AvatarFallback>AD</AvatarFallback>
      </AvatarRoot>
    );

    const root = screen.getByTestId('avatar');
    expect(root).toHaveAttribute('id', 'user-avatar');
    expect(root).toHaveAttribute('aria-label', 'Ada Lovelace');
    expect(root).toHaveAttribute('data-slot', 'avatar');
  });

  it('forwards refs on the root and the fallback', () => {
    const rootRef = createRef<HTMLSpanElement>();
    const fallbackRef = createRef<HTMLSpanElement>();
    render(
      <AvatarRoot ref={rootRef} data-testid="avatar">
        <AvatarFallback ref={fallbackRef}>AD</AvatarFallback>
      </AvatarRoot>
    );

    expect(rootRef.current).toBe(screen.getByTestId('avatar'));
    expect(fallbackRef.current).toBe(screen.getByText('AD'));
  });

  /**
   * There is no `size` prop by design — the caller sizes the box. This pins
   * that a caller's dimensions actually beat the default rather than emitting
   * two competing utilities.
   */
  it('lets a caller size the box, overriding the default dimensions', () => {
    render(
      <AvatarRoot className="h-16 w-16" data-testid="avatar">
        <AvatarFallback>AD</AvatarFallback>
      </AvatarRoot>
    );

    const className = screen.getByTestId('avatar').className;
    expect(className).toContain('h-16');
    expect(className).toContain('w-16');
    expect(className).not.toContain('h-9');
    expect(className).not.toContain('w-9');
  });

  it('never renders a raw palette colour on any slot', () => {
    const { container } = render(
      <AvatarRoot>
        <AvatarImage src="https://example.test/a.png" alt="Ada" />
        <AvatarFallback>AD</AvatarFallback>
      </AvatarRoot>
    );

    for (const el of Array.from(container.querySelectorAll('[data-slot^="avatar"]'))) {
      expect(el.className, el.getAttribute('data-slot') ?? '').not.toMatch(RAW_PALETTE);
    }
  });
});
