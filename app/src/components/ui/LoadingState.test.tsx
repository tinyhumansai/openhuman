import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { CenteredLoadingState, ErrorBanner, InlineLoadingStatus, Spinner } from './LoadingState';

describe('ErrorBanner', () => {
  it('retains message call sites and exposes errors as alerts', () => {
    render(<ErrorBanner message="Could not load" />);

    expect(screen.getByRole('alert')).toHaveTextContent('Could not load');
  });

  it('accepts React content and an optional action', () => {
    render(
      <ErrorBanner action={<button type="button">Retry</button>}>
        <strong>Connection lost</strong>
      </ErrorBanner>
    );

    const alert = screen.getByRole('alert');
    expect(alert).toContainElement(screen.getByText('Connection lost'));
    expect(alert).toContainElement(screen.getByRole('button', { name: 'Retry' }));
  });

  it('supports small and medium presentation sizes', () => {
    const { rerender } = render(<ErrorBanner message="Small error" size="sm" />);
    expect(screen.getByRole('alert')).toHaveClass('p-3');
    expect(screen.getByRole('alert')).toHaveClass('text-xs');

    rerender(<ErrorBanner message="Medium error" size="md" />);
    expect(screen.getByRole('alert')).toHaveClass('p-4');
    expect(screen.getByRole('alert')).toHaveClass('text-sm');
  });
});

describe('loading states', () => {
  it('renders inline and centered labels with their shared spinners', () => {
    const { container } = render(
      <>
        <InlineLoadingStatus label="Checking" />
        <CenteredLoadingState label="Loading runs" />
      </>
    );

    expect(screen.getByText('Checking')).toBeInTheDocument();
    expect(screen.getByText('Loading runs')).toBeInTheDocument();
    expect(container.querySelectorAll('svg')).toHaveLength(2);
  });
});

describe('Spinner', () => {
  // There is no `ui/Spinner.tsx` on purpose — this module re-exports the one
  // SVG in `ui/icons.tsx`. These cases pin that the alias resolves and that the
  // spinner stays themeable, so a second implementation is never needed.
  it('renders the shared spinner and forwards rest props and data-testid', () => {
    render(<Spinner data-testid="spinner" aria-label="Loading" role="img" />);

    const spinner = screen.getByTestId('spinner');
    expect(spinner.tagName).toBe('svg');
    expect(spinner).toHaveAttribute('aria-label', 'Loading');
    expect(spinner).toHaveAttribute('role', 'img');
    expect(spinner.getAttribute('class')).toMatch(/animate-spin/);
  });

  it('is sized by className and uses no raw palette utility', () => {
    render(<Spinner data-testid="spinner" className="h-5 w-5 text-content-muted" />);

    const className = screen.getByTestId('spinner').getAttribute('class') ?? '';
    expect(className).toMatch(/h-5 w-5/);
    expect(className).not.toMatch(
      /\b(bg|text|border|ring)-(neutral|stone|slate|canvas|white|black)\b/
    );
  });
});
