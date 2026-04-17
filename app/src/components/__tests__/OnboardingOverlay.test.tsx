import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import OnboardingOverlay from '../OnboardingOverlay';

const mockUseCoreState = vi.fn();
const mockNavigate = vi.fn();
let mockPathname = '/';

vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual<typeof import('react-router-dom')>('react-router-dom');
  return {
    ...actual,
    useNavigate: () => mockNavigate,
    useLocation: () => ({ pathname: mockPathname, search: '', hash: '', state: null, key: 'test' }),
  };
});

vi.mock('../../providers/CoreStateProvider', () => ({ useCoreState: () => mockUseCoreState() }));

vi.mock('../../pages/onboarding/Onboarding', () => ({
  default: ({ onComplete }: { onComplete: () => void }) => (
    <div>
      <button onClick={onComplete}>Complete</button>
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
    mockNavigate.mockReset();
    mockPathname = '/';
  });

  it('does not render when onboarding is completed', () => {
    mockUseCoreState.mockReturnValue(makeCoreState({ onboardingCompleted: true }));

    render(
      <MemoryRouter>
        <OnboardingOverlay />
      </MemoryRouter>
    );

    expect(screen.queryByText('Complete')).not.toBeInTheDocument();
  });

  it('does not render when no token', () => {
    mockUseCoreState.mockReturnValue(makeCoreState({ sessionToken: null }));

    render(
      <MemoryRouter>
        <OnboardingOverlay />
      </MemoryRouter>
    );

    expect(screen.queryByText('Complete')).not.toBeInTheDocument();
  });

  it('does not render when user profile is not loaded yet', () => {
    mockUseCoreState.mockReturnValue(makeCoreState({ currentUser: {} }));

    render(
      <MemoryRouter>
        <OnboardingOverlay />
      </MemoryRouter>
    );

    expect(screen.queryByText('Complete')).not.toBeInTheDocument();
  });

  it('renders when the user is authenticated and onboarding is incomplete', () => {
    mockUseCoreState.mockReturnValue(makeCoreState());

    render(
      <MemoryRouter>
        <OnboardingOverlay />
      </MemoryRouter>
    );

    expect(screen.getByText('Complete')).toBeInTheDocument();
  });

  it('does not render while bootstrapping', () => {
    mockUseCoreState.mockReturnValue({ ...makeCoreState(), isBootstrapping: true });

    render(
      <MemoryRouter>
        <OnboardingOverlay />
      </MemoryRouter>
    );

    expect(screen.queryByText('Complete')).not.toBeInTheDocument();
  });

  it('navigates to chat and persists onboarding completion on finish', async () => {
    const coreState = makeCoreState();
    mockUseCoreState.mockReturnValue(coreState);

    render(
      <MemoryRouter>
        <OnboardingOverlay />
      </MemoryRouter>
    );

    fireEvent.click(screen.getByText('Complete'));

    await waitFor(() => {
      expect(mockNavigate).toHaveBeenCalledWith('/chat', { replace: true });
      expect(coreState.setOnboardingCompletedFlag).toHaveBeenCalledWith(true);
    });
  });

  it('drops dismissing mask after chat route becomes active', async () => {
    const coreState = makeCoreState();
    mockUseCoreState.mockReturnValue(coreState);

    const { rerender } = render(
      <MemoryRouter>
        <OnboardingOverlay />
      </MemoryRouter>
    );

    fireEvent.click(screen.getByText('Complete'));
    await waitFor(() => {
      expect(mockNavigate).toHaveBeenCalledWith('/chat', { replace: true });
    });
    expect(screen.getByText('Complete')).toBeInTheDocument();

    mockPathname = '/chat';
    mockUseCoreState.mockReturnValue(makeCoreState({ onboardingCompleted: true }));
    rerender(
      <MemoryRouter>
        <OnboardingOverlay />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.queryByText('Complete')).not.toBeInTheDocument();
    });
  });
});
