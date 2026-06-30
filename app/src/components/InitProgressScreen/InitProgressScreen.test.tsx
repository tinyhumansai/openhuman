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
        id: 'python_runtime',
        label: 'Python runtime',
        required: false,
        state: 'done',
        message: null,
        percent: 100,
        updatedAt: null,
      },
      {
        id: 'spacy',
        label: 'spaCy',
        required: false,
        state: 'running',
        message: null,
        percent: null,
        updatedAt: null,
      },
      {
        id: 'node_runtime',
        label: 'Node.js runtime',
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

    expect(screen.getByText('Python runtime')).toBeInTheDocument();
    expect(screen.getByText('Language model')).toBeInTheDocument();
    expect(screen.getByText('Node.js runtime')).toBeInTheDocument();
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
          id: 'spacy',
          label: 'spaCy',
          required: false,
          state: 'failed',
          message: 'pip install timed out',
          percent: null,
          updatedAt: null,
        },
      ],
    });

    renderWithProviders(
      <InitProgressScreen snapshot={failed} onRetry={onRetry} onContinue={onContinue} />
    );

    expect(screen.getByText('pip install timed out')).toBeInTheDocument();

    fireEvent.click(screen.getByText('Retry'));
    expect(onRetry).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByText('Continue anyway'));
    expect(onContinue).toHaveBeenCalledTimes(1);
  });
});
