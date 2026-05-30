/**
 * Verifies that `ContextPage` forwards a `completeAndExit` rejection into
 * Sentry with the documented tags (#2081). The handled `.catch` would
 * otherwise keep the failure out of `Sentry.globalHandlersIntegration`, so
 * an explicit capture is the only way the dashboard sees it.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import { OnboardingContext, type OnboardingContextValue } from '../../OnboardingContext';
import ContextPage from '../ContextPage';

const hoisted = vi.hoisted(() => ({ captureException: vi.fn() }));

vi.mock('@sentry/react', () => ({ captureException: hoisted.captureException }));

// `trackEvent` writes through to analytics → react-ga4 / Sentry. Stub it so
// the rejection-capture assertion isn't entangled with consent / GA wiring.
vi.mock('../../../../services/analytics', () => ({ trackEvent: vi.fn() }));

// The page renders `ContextGatheringStep`, which auto-starts a heavy pipeline
// on mount when `connectedSources` includes a Composio Gmail entry. Stub it
// to a button that simply invokes `onNext` so the failure path is reachable
// without simulating the full pipeline.
vi.mock('../../steps/ContextGatheringStep', () => ({
  default: ({ onNext }: { onNext: () => void | Promise<void> }) => (
    <button data-testid="continue-to-chat" onClick={() => onNext()}>
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

describe('ContextPage — onboarding-complete Sentry capture (#2081)', () => {
  beforeEach(() => {
    hoisted.captureException.mockReset();
  });

  it('captures a completeAndExit rejection to Sentry with the documented tags', async () => {
    const failure = new Error('app_state_snapshot timed out');
    const completeAndExit = vi.fn().mockRejectedValue(failure);

    renderContextPage(completeAndExit);
    fireEvent.click(screen.getByTestId('continue-to-chat'));

    await waitFor(() => expect(hoisted.captureException).toHaveBeenCalledTimes(1));
    expect(completeAndExit).toHaveBeenCalledTimes(1);

    const [thrown, ctx] = hoisted.captureException.mock.calls[0];
    expect(thrown).toBe(failure);
    expect(ctx).toEqual({ tags: { flow: 'onboarding-complete', step: 'continue-to-chat' } });
  });

  it('does not capture anything when completeAndExit resolves', async () => {
    const completeAndExit = vi.fn().mockResolvedValue(undefined);

    renderContextPage(completeAndExit);
    fireEvent.click(screen.getByTestId('continue-to-chat'));

    // Let the resolved promise settle.
    await waitFor(() => expect(completeAndExit).toHaveBeenCalledTimes(1));
    expect(hoisted.captureException).not.toHaveBeenCalled();
  });
});
