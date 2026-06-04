import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import RuntimeChoicePage from './RuntimeChoicePage';

const navigateMock = vi.hoisted(() => vi.fn());
const setDraftMock = vi.hoisted(() => vi.fn());
const completeAndExitMock = vi.hoisted(() => vi.fn());
const isLocalSessionTokenMock = vi.hoisted(() => vi.fn(() => false));

vi.mock('react-router-dom', async importOriginal => {
  const actual = await importOriginal<typeof import('react-router-dom')>();
  return { ...actual, useNavigate: () => navigateMock };
});

vi.mock('../../../utils/localSession', () => ({ isLocalSessionToken: isLocalSessionTokenMock }));

vi.mock('../../../services/analytics', () => ({ trackEvent: vi.fn() }));

vi.mock('../OnboardingContext', () => ({
  useOnboardingContext: () => ({ setDraft: setDraftMock, completeAndExit: completeAndExitMock }),
}));

vi.mock('../steps/RuntimeChoiceStep', () => ({
  default: ({ onNext }: { onNext: (mode: string) => void }) => (
    <div data-testid="runtime-choice-step">
      <button onClick={() => onNext('cloud')}>Pick Cloud</button>
      <button onClick={() => onNext('custom')}>Pick Custom</button>
    </div>
  ),
}));

describe('RuntimeChoicePage', () => {
  beforeEach(() => {
    navigateMock.mockReset();
    setDraftMock.mockReset();
    completeAndExitMock.mockReset();
    isLocalSessionTokenMock.mockReturnValue(false);
  });

  it('redirects to custom inference and renders nothing for local sessions', () => {
    isLocalSessionTokenMock.mockReturnValue(true);
    const { container } = renderWithProviders(<RuntimeChoicePage />);
    expect(navigateMock).toHaveBeenCalledWith('/onboarding/custom/inference', { replace: true });
    expect(container.firstChild).toBeNull();
  });

  it('renders the choice step for cloud sessions', () => {
    renderWithProviders(<RuntimeChoicePage />);
    expect(screen.getByTestId('runtime-choice-step')).toBeInTheDocument();
  });

  it('navigates to custom inference when custom mode is picked', async () => {
    renderWithProviders(<RuntimeChoicePage />);
    fireEvent.click(screen.getByRole('button', { name: 'Pick Custom' }));
    expect(setDraftMock).toHaveBeenCalled();
    expect(navigateMock).toHaveBeenCalledWith('/onboarding/custom/inference');
    expect(completeAndExitMock).not.toHaveBeenCalled();
  });

  it('calls completeAndExit for cloud mode and shows no error on success', async () => {
    completeAndExitMock.mockResolvedValue(undefined);
    renderWithProviders(<RuntimeChoicePage />);
    fireEvent.click(screen.getByRole('button', { name: 'Pick Cloud' }));
    await waitFor(() => expect(completeAndExitMock).toHaveBeenCalledTimes(1));
    expect(screen.queryByTestId('onboarding-runtime-choice-exit-error')).not.toBeInTheDocument();
  });

  it('shows error banner when completeAndExit throws', async () => {
    completeAndExitMock.mockRejectedValue(new Error('network failure'));
    renderWithProviders(<RuntimeChoicePage />);
    fireEvent.click(screen.getByRole('button', { name: 'Pick Cloud' }));
    await waitFor(() =>
      expect(screen.getByTestId('onboarding-runtime-choice-exit-error')).toBeInTheDocument()
    );
  });
});
