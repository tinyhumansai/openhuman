import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { WalkthroughState } from '../../../pages/onboarding/OnboardingContext';
import { renderWithProviders } from '../../../test/test-utils';
import WalkthroughPhasePanel from '../WalkthroughPhasePanel';
import { WalkthroughProvider } from '../WalkthroughProvider';

function renderPanel(state: WalkthroughState, onAdvance = vi.fn(), onSkip = vi.fn()) {
  renderWithProviders(
    <WalkthroughProvider state={state} onAdvance={onAdvance} onSkip={onSkip}>
      <WalkthroughPhasePanel />
    </WalkthroughProvider>
  );
  return { onAdvance, onSkip };
}

describe('WalkthroughPhasePanel', () => {
  it('renders completed summary cards during review', () => {
    renderPanel({
      phase: 'review',
      steps: [
        { key: 'briefings', completed: true },
        { key: 'summaries', completed: true },
      ],
      completed: false,
      skipped: false,
    });

    expect(screen.getByText('Daily Briefings')).toBeInTheDocument();
    expect(screen.getByText('Meeting Summaries')).toBeInTheDocument();
    expect(screen.queryByText('No items reviewed yet.')).not.toBeInTheDocument();
  });

  it('finishes the review phase through the provider advance callback', () => {
    const { onAdvance } = renderPanel({
      phase: 'review',
      steps: [],
      completed: false,
      skipped: true,
    });

    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));
    expect(onAdvance).toHaveBeenCalledWith();
  });

  it('renders active phase cards as informational and advances with continue', () => {
    const { onAdvance } = renderPanel({
      phase: 'connect',
      steps: [{ key: 'gmail', completed: false }],
      completed: false,
      skipped: false,
    });

    expect(screen.queryByRole('button', { name: /Gmail/i })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));
    expect(onAdvance).toHaveBeenCalledWith();
  });

  it('uses localized status suffixes in progress labels', () => {
    renderPanel({
      phase: 'connect',
      steps: [{ key: 'gmail', completed: false }],
      completed: false,
      skipped: false,
    });

    expect(screen.getByLabelText('Welcome (completed)')).toBeInTheDocument();
    expect(screen.getByLabelText('Connect (current)')).toBeInTheDocument();
  });
});
