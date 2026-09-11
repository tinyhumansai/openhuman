import { render, screen } from '@testing-library/react';
import { createRef } from 'react';
import { describe, expect, it } from 'vitest';

import VisuallyHidden from './VisuallyHidden';

const RAW_PALETTE = /\b(bg|text|border|ring)-(neutral|stone|slate|canvas|white|black)\b/;

describe('VisuallyHidden', () => {
  it('renders its children into the accessibility tree', () => {
    render(
      <VisuallyHidden>
        <h2>Close settings</h2>
      </VisuallyHidden>
    );
    // Present for assistive technology — `display: none` would not be.
    expect(screen.getByRole('heading', { name: 'Close settings' })).toBeInTheDocument();
  });

  it('forwards ...rest and data-testid onto the DOM node', () => {
    render(
      <VisuallyHidden data-testid="vh" id="dialog-title" aria-live="polite">
        Saving
      </VisuallyHidden>
    );
    const node = screen.getByTestId('vh');
    expect(node).toHaveAttribute('data-slot', 'visually-hidden');
    expect(node).toHaveAttribute('id', 'dialog-title');
    expect(node).toHaveAttribute('aria-live', 'polite');
  });

  it('clips itself out of the visual layout rather than hiding it', () => {
    render(<VisuallyHidden data-testid="vh">Hidden label</VisuallyHidden>);
    const node = screen.getByTestId('vh');
    expect(node.style.position).toBe('absolute');
    expect(node.style.display).not.toBe('none');
  });

  it('forwards a ref', () => {
    const ref = createRef<HTMLSpanElement>();
    render(
      <VisuallyHidden ref={ref} data-testid="vh">
        Hidden
      </VisuallyHidden>
    );
    expect(ref.current).toBe(screen.getByTestId('vh'));
  });

  it('lets a caller className through and emits no raw palette utility', () => {
    render(
      <VisuallyHidden data-testid="vh" className="mt-2">
        Hidden
      </VisuallyHidden>
    );
    const node = screen.getByTestId('vh');
    expect(node.className).toContain('mt-2');
    expect(node.className).not.toMatch(RAW_PALETTE);
  });
});
