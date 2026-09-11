import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import ContentSurface from './ContentSurface';

describe('ContentSurface', () => {
  afterEach(cleanup);

  it('renders children', () => {
    render(
      <ContentSurface>
        <p>routed page</p>
      </ContentSurface>
    );
    expect(screen.getByText('routed page')).toBeTruthy();
  });

  it('frames the surface as an inset rounded card by default', () => {
    render(<ContentSurface>body</ContentSurface>);
    const surface = screen.getByTestId('app-content-surface');
    expect(surface.className).toContain('rounded-2xl');
    expect(surface.className).toContain('shadow-content-edge');
    // Inset on three sides. The left edge is deliberately flush: the card butts
    // against the sidebar column, so the shell's gutter is top/right/bottom.
    expect(surface.className).toContain('my-3');
    expect(surface.className).toContain('mr-3');
    expect(surface.className).not.toContain('ml-3');
    expect(surface.dataset.unframed).toBeUndefined();
  });

  it('drops the radius, insets and seam when unframed', () => {
    render(<ContentSurface unframed>body</ContentSurface>);
    const surface = screen.getByTestId('app-content-surface');
    // The compositing constraint this exists for: content drawn above the HTML
    // layer as a plain rectangle would leave four square corners poking through
    // a rounded card.
    expect(surface.className).not.toContain('rounded-2xl');
    expect(surface.className).not.toContain('shadow-content-edge');
    expect(surface.className).not.toContain('m-3');
    expect(surface.dataset.unframed).toBe('true');
  });

  it('keeps the bounded flex column in both modes so the page owns the scroll', () => {
    const { rerender } = render(<ContentSurface>body</ContentSurface>);
    expect(screen.getByTestId('app-content-surface').className).toContain('min-h-0');
    rerender(<ContentSurface unframed>body</ContentSurface>);
    const surface = screen.getByTestId('app-content-surface');
    expect(surface.className).toContain('min-h-0');
    expect(surface.className).toContain('flex-1');
    expect(surface.className).toContain('bg-surface');
  });
});
