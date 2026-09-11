import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { createRef } from 'react';
import { describe, expect, it, vi } from 'vitest';

import Toggle from './Toggle';

const RAW_PALETTE = /\b(bg|text|border|ring)-(neutral|stone|slate|canvas|white|black)\b/;

describe('Toggle', () => {
  it('renders a pressed-state button', () => {
    render(<Toggle aria-label="Bold">B</Toggle>);
    const button = screen.getByRole('button', { name: 'Bold' });
    expect(button).toHaveAttribute('aria-pressed', 'false');
    expect(button).toHaveAttribute('data-slot', 'toggle');
    expect(button).toHaveAttribute('data-state', 'off');
  });

  it('forwards ...rest, data-testid and analyticsId', () => {
    render(<Toggle aria-label="Bold" data-testid="bold" analyticsId="editor-bold" name="bold" />);
    const button = screen.getByTestId('bold');
    expect(button).toHaveAttribute('data-analytics-id', 'editor-bold');
    expect(button).toHaveAttribute('name', 'bold');
  });

  it('forwards a ref', () => {
    const ref = createRef<HTMLButtonElement>();
    render(<Toggle ref={ref} aria-label="Bold" />);
    expect(ref.current).toBeInstanceOf(HTMLButtonElement);
  });

  it('toggles on Enter and on Space', async () => {
    const user = userEvent.setup();
    const onPressedChange = vi.fn();
    render(<Toggle aria-label="Bold" onPressedChange={onPressedChange} data-testid="bold" />);
    const button = screen.getByTestId('bold');

    button.focus();
    await user.keyboard('{Enter}');
    expect(button).toHaveAttribute('aria-pressed', 'true');
    expect(button).toHaveAttribute('data-state', 'on');

    await user.keyboard(' ');
    expect(button).toHaveAttribute('aria-pressed', 'false');

    expect(onPressedChange).toHaveBeenNthCalledWith(1, true);
    expect(onPressedChange).toHaveBeenNthCalledWith(2, false);
  });

  it('respects a controlled pressed prop', () => {
    render(<Toggle aria-label="Bold" pressed data-testid="bold" />);
    expect(screen.getByTestId('bold')).toHaveAttribute('data-state', 'on');
  });

  it('tags the variant, tone and size axes', () => {
    render(
      <Toggle aria-label="Mute" variant="secondary" tone="danger" size="xl" data-testid="mute" />
    );
    const button = screen.getByTestId('mute');
    expect(button).toHaveAttribute('data-variant', 'secondary');
    expect(button).toHaveAttribute('data-tone', 'danger');
    expect(button).toHaveAttribute('data-size', 'xl');
  });

  it('lets a caller className win over the size default', () => {
    render(<Toggle aria-label="Bold" size="md" className="h-20" data-testid="bold" />);
    const cls = screen.getByTestId('bold').className;
    expect(cls).toContain('h-20');
    expect(cls).not.toContain('h-9');
  });

  it('does not emit a raw palette utility in any variant/tone pairing', () => {
    for (const variant of ['secondary', 'tertiary'] as const) {
      for (const tone of ['default', 'danger'] as const) {
        const { unmount } = render(
          <Toggle aria-label="X" variant={variant} tone={tone} data-testid="t" />
        );
        expect(screen.getByTestId('t').className).not.toMatch(RAW_PALETTE);
        unmount();
      }
    }
  });
});
