import { fireEvent, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import { OnboardingContext, type OnboardingContextValue } from '../../OnboardingContext';
import ContextPage from '../ContextPage';

const hoisted = vi.hoisted(() => ({ lastOnNextResult: undefined as unknown, trackEvent: vi.fn() }));

vi.mock('../../../../services/analytics', () => ({ trackEvent: hoisted.trackEvent }));

vi.mock('../../steps/ContextGatheringStep', () => ({
  default: ({ onNext }: { onNext: () => void | Promise<void> }) => (
    <button
      data-testid="continue-to-chat"
      onClick={() => {
        hoisted.lastOnNextResult = onNext();
      }}>
      Continue to chat
    </button>
  ),
}));

function renderContextPage(completeAndExit: OnboardingContextValue['completeAndExit']) {
  const value: OnboardingContextValue = {
    draft: { connectedSources: ['composio:gmail'] },
    setDraft: vi.fn(),
    completeAndExit,
  };
  return renderWithProviders(
    <OnboardingContext.Provider value={value}>
      <ContextPage />
    </OnboardingContext.Provider>
  );
}

describe('ContextPage', () => {
  beforeEach(() => {
    hoisted.lastOnNextResult = undefined;
    hoisted.trackEvent.mockReset();
  });

  it('returns the completeAndExit promise so the step can show click feedback', async () => {
    const failure = new Error('app_state_snapshot timed out');
    const completeAndExit = vi.fn().mockRejectedValue(failure);

    renderContextPage(completeAndExit);
    fireEvent.click(screen.getByTestId('continue-to-chat'));

    expect(hoisted.trackEvent).toHaveBeenCalledWith('onboarding_step_complete', {
      step_name: 'context',
    });
    expect(completeAndExit).toHaveBeenCalledTimes(1);
    await expect(hoisted.lastOnNextResult).rejects.toBe(failure);
  });
});
