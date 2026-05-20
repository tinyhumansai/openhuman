import { render, screen } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

import DefaultRedirect from '../DefaultRedirect';

vi.mock('../../utils/config', () => ({ DEV_FORCE_ONBOARDING: false }));

const mockUseCoreState = vi.fn();
vi.mock('../../providers/CoreStateProvider', () => ({ useCoreState: () => mockUseCoreState() }));

function renderRedirect(initialEntry = '*') {
  return render(
    <MemoryRouter initialEntries={[`/${initialEntry}`]}>
      <Routes>
        <Route path="/" element={<div>Welcome</div>} />
        <Route path="/onboarding" element={<div>Onboarding</div>} />
        <Route path="/home" element={<div>Home</div>} />
        <Route path="*" element={<DefaultRedirect />} />
      </Routes>
    </MemoryRouter>
  );
}

describe('DefaultRedirect', () => {
  it('shows loading while bootstrapping', () => {
    mockUseCoreState.mockReturnValue({
      isBootstrapping: true,
      snapshot: { sessionToken: null, currentUser: null, onboardingCompleted: false },
    });

    renderRedirect();

    expect(screen.queryByText('Welcome')).not.toBeInTheDocument();
    expect(screen.queryByText('Onboarding')).not.toBeInTheDocument();
    expect(screen.queryByText('Home')).not.toBeInTheDocument();
  });

  it('redirects to / when no session token', () => {
    mockUseCoreState.mockReturnValue({
      isBootstrapping: false,
      snapshot: { sessionToken: null, currentUser: null, onboardingCompleted: false },
    });

    renderRedirect();

    expect(screen.getByText('Welcome')).toBeInTheDocument();
  });

  it('shows loading when session token arrived but currentUser is not yet set (post-login race)', () => {
    // This is the race: token set by core-state:session-token-updated but
    // refresh() hasn't resolved yet — currentUser is still null from
    // toSignedOutSnapshot(), onboardingCompleted is still false.
    mockUseCoreState.mockReturnValue({
      isBootstrapping: false,
      snapshot: { sessionToken: 'token-abc', currentUser: null, onboardingCompleted: false },
    });

    renderRedirect();

    // Must NOT navigate to /onboarding — that would be the stale-snapshot bug
    expect(screen.queryByText('Onboarding')).not.toBeInTheDocument();
    expect(screen.queryByText('Home')).not.toBeInTheDocument();
    expect(screen.queryByText('Welcome')).not.toBeInTheDocument();
    // Positively assert the loading screen rendered (not just "nothing visible")
    expect(screen.getByText('Initializing OpenHuman...')).toBeInTheDocument();
  });

  it('redirects to /onboarding for a genuinely new user (currentUser set, onboarding false)', () => {
    mockUseCoreState.mockReturnValue({
      isBootstrapping: false,
      snapshot: {
        sessionToken: 'token-abc',
        currentUser: { _id: 'user-1', email: 'new@test.com' },
        onboardingCompleted: false,
      },
    });

    renderRedirect();

    expect(screen.getByText('Onboarding')).toBeInTheDocument();
  });

  it('redirects to /home for a returning user who already completed onboarding', () => {
    mockUseCoreState.mockReturnValue({
      isBootstrapping: false,
      snapshot: {
        sessionToken: 'token-abc',
        currentUser: { _id: 'user-1', email: 'returning@test.com' },
        onboardingCompleted: true,
      },
    });

    renderRedirect();

    expect(screen.getByText('Home')).toBeInTheDocument();
  });
});
