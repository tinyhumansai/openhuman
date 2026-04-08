import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import OnboardingOverlay from '../OnboardingOverlay';

const mockUseCoreState = vi.fn();

vi.mock('../../providers/CoreStateProvider', () => ({ useCoreState: () => mockUseCoreState() }));

vi.mock('../../pages/onboarding/Onboarding', () => ({
  default: ({ onComplete }: { onComplete: () => void }) => (
    <div>
      <button onClick={onComplete}>Skip</button>
    </div>
  ),
}));

function makeCoreState(overrides?: Record<string, unknown>) {
  return {
    isBootstrapping: false,
    snapshot: {
      sessionToken: 'test-jwt',
      currentUser: { _id: 'user-1', username: 'tester', firstName: 'Test' },
      onboardingCompleted: false,
      ...overrides,
    },
    setOnboardingCompletedFlag: vi.fn().mockResolvedValue(undefined),
  };
}

describe('OnboardingOverlay', () => {
  beforeEach(() => {
    mockUseCoreState.mockReset();
  });

  it('does not render when onboarding is completed', () => {
    mockUseCoreState.mockReturnValue(makeCoreState({ onboardingCompleted: true }));

    render(
      <MemoryRouter>
        <OnboardingOverlay />
      </MemoryRouter>
    );

    expect(screen.queryByText('Skip')).not.toBeInTheDocument();
  });

  it('does not render when no token', () => {
    mockUseCoreState.mockReturnValue(makeCoreState({ sessionToken: null }));

    render(
      <MemoryRouter>
        <OnboardingOverlay />
      </MemoryRouter>
    );

    expect(screen.queryByText('Skip')).not.toBeInTheDocument();
  });

  it('does not render when user profile is not loaded yet', () => {
    mockUseCoreState.mockReturnValue(makeCoreState({ currentUser: {} }));

    render(
      <MemoryRouter>
        <OnboardingOverlay />
      </MemoryRouter>
    );

    expect(screen.queryByText('Skip')).not.toBeInTheDocument();
  });

  it('renders when the user is authenticated and onboarding is incomplete', () => {
    mockUseCoreState.mockReturnValue(makeCoreState());

    render(
      <MemoryRouter>
        <OnboardingOverlay />
      </MemoryRouter>
    );

    expect(screen.getByText('Skip')).toBeInTheDocument();
  });

  it('does not render while bootstrapping', () => {
    mockUseCoreState.mockReturnValue({ ...makeCoreState(), isBootstrapping: true });

    render(
      <MemoryRouter>
        <OnboardingOverlay />
      </MemoryRouter>
    );

    expect(screen.queryByText('Skip')).not.toBeInTheDocument();
  });
});
