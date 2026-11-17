import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { ToolError } from './tool-error';

const failure = {
  name: 'read_file',
  target: 'notes.txt',
  message: 'File is unavailable',
  retrying: false,
};

describe('ToolError', () => {
  it.each(['retry', 'skip'] as const)('shows only the supported %s recovery action', action => {
    const onAction = vi.fn();
    render(
      <ToolError
        {...failure}
        {...(action === 'retry' ? { onRetry: onAction } : { onSkip: onAction })}
      />
    );
    const buttons = screen.getAllByRole('button');
    expect(buttons).toHaveLength(1);
    expect(buttons[0]).toHaveAccessibleName(action === 'retry' ? 'Retry' : 'Skip');
    // All application actions use the canonical UI Button and tertiary hierarchy.
    expect(buttons[0]).toHaveAttribute('data-slot', 'button');
    expect(buttons[0]).toHaveAttribute('data-variant', 'tertiary');
    fireEvent.click(buttons[0]);
    expect(onAction).toHaveBeenCalledOnce();
  });

  it('shows the failed action, its cause, and the available recovery actions', () => {
    const onRetry = vi.fn();
    const onSkip = vi.fn();
    render(
      <ToolError {...failure} attempt={2} maxAttempts={3} onRetry={onRetry} onSkip={onSkip} />
    );
    expect(screen.getByText('read_file')).toBeVisible();
    expect(screen.getByText('notes.txt')).toBeVisible();
    expect(screen.getByText('File is unavailable')).toBeVisible();
    expect(screen.getByText('2/3')).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    fireEvent.click(screen.getByRole('button', { name: 'Skip' }));
    expect(onRetry).toHaveBeenCalledOnce();
    expect(onSkip).toHaveBeenCalledOnce();
  });

  it('prevents duplicate retries while allowing the user to skip', () => {
    const onRetry = vi.fn();
    const onSkip = vi.fn();
    const view = render(
      <ToolError
        {...failure}
        retryLabel="Try again"
        retryingLabel="Trying again"
        skipLabel="Continue"
        onRetry={onRetry}
        onSkip={onSkip}
      />
    );
    expect(screen.getByRole('button', { name: 'Try again' })).toBeEnabled();
    view.rerender(
      <ToolError
        {...failure}
        retrying
        retryingLabel="Trying again"
        skipLabel="Continue"
        onRetry={onRetry}
        onSkip={onSkip}
      />
    );
    const retry = screen.getByRole('button', { name: 'Trying again' });
    expect(retry).toBeDisabled();
    fireEvent.click(retry);
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));
    expect(onRetry).not.toHaveBeenCalled();
    expect(onSkip).toHaveBeenCalledOnce();
  });

  it('renders a read-only failure without controls when recovery is unavailable', () => {
    render(<ToolError {...failure} attempt={1} aria-label="Tool failed" />);
    expect(screen.getByLabelText('Tool failed')).toBeVisible();
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
    expect(screen.queryByText('1/')).not.toBeInTheDocument();
  });
});
