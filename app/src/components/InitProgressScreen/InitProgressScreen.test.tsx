import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { HarnessInitSnapshot } from '../../services/harnessInitService';
import { renderWithProviders } from '../../test/test-utils';
import InitProgressScreen from './InitProgressScreen';

function snapshot(overrides: Partial<HarnessInitSnapshot> = {}): HarnessInitSnapshot {
  return {
    overall: 'running',
    startedAt: null,
    finishedAt: null,
    steps: [
      {
        id: 'first_check',
        label: 'First check',
        required: false,
        state: 'done',
        message: null,
        percent: 100,
        updatedAt: null,
      },
      {
        id: 'second_check',
        label: 'Second check',
        required: false,
        state: 'running',
        message: null,
        percent: null,
        updatedAt: null,
      },
      {
        id: 'third_check',
        label: 'Third check',
        required: false,
        state: 'pending',
        message: null,
        percent: null,
        updatedAt: null,
      },
    ],
    ...overrides,
  };
}

describe('InitProgressScreen', () => {
  it('renders each step with its localized label and state', () => {
    renderWithProviders(
      <InitProgressScreen snapshot={snapshot()} onRetry={vi.fn()} onContinue={vi.fn()} />
    );

    expect(screen.getByText('First check')).toBeInTheDocument();
    expect(screen.getByText('Second check')).toBeInTheDocument();
    expect(screen.getByText('Third check')).toBeInTheDocument();
    expect(screen.getByText('Ready')).toBeInTheDocument();
    expect(screen.getByText('Installing…')).toBeInTheDocument();
    expect(screen.getByText('Waiting')).toBeInTheDocument();
    // No failure actions while running.
    expect(screen.queryByText('Retry')).not.toBeInTheDocument();
  });

  it('offers a Run in background action while running', () => {
    const onContinue = vi.fn();
    renderWithProviders(
      <InitProgressScreen snapshot={snapshot()} onRetry={vi.fn()} onContinue={onContinue} />
    );

    expect(screen.getByTestId('harness-init-background')).toBeInTheDocument();
    expect(screen.queryByTestId('harness-init-continue-anyway')).not.toBeInTheDocument();
    fireEvent.click(screen.getByText('Run in background'));
    expect(onContinue).toHaveBeenCalledTimes(1);
  });

  it('shows the failing message and Retry / Continue on a failed run', () => {
    const onRetry = vi.fn();
    const onContinue = vi.fn();
    const failed = snapshot({
      overall: 'failed',
      steps: [
        {
          id: 'second_check',
          label: 'Second check',
          required: false,
          state: 'failed',
          message: 'Initialization step failed',
          percent: null,
          updatedAt: null,
        },
      ],
    });

    renderWithProviders(
      <InitProgressScreen snapshot={failed} onRetry={onRetry} onContinue={onContinue} />
    );

    expect(screen.getByText('Initialization step failed')).toBeInTheDocument();
    expect(screen.getByTestId('harness-init-continue-anyway')).toBeInTheDocument();
    expect(screen.queryByTestId('harness-init-background')).not.toBeInTheDocument();

    fireEvent.click(screen.getByText('Retry'));
    expect(onRetry).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByText('Continue anyway'));
    expect(onContinue).toHaveBeenCalledTimes(1);
  });
});
