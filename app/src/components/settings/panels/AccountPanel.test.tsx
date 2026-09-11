import { screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import AccountPanel from './AccountPanel';

const useCoreStateMock = vi.fn();
vi.mock('../../../providers/CoreStateProvider', () => ({ useCoreState: () => useCoreStateMock() }));

vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: vi.fn(),
    navigateToTeamManagement: vi.fn(),
    breadcrumbs: [],
  }),
}));

function renderPanel(currentUser: Record<string, unknown> | null) {
  useCoreStateMock.mockReturnValue({
    snapshot: { currentUser, auth: { userId: currentUser?._id ?? null } },
    clearSession: vi.fn(),
  });
  return renderWithProviders(<AccountPanel />, { preloadedState: { locale: { current: 'en' } } });
}

describe('AccountPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders the avatar initial and full name for a signed-in user', () => {
    renderPanel({ firstName: 'Ada', lastName: 'Lovelace', username: 'ada' });

    expect(screen.getByText('Ada Lovelace')).toBeInTheDocument();
    expect(screen.getByText('@ada')).toBeInTheDocument();
    // Avatar fallback renders the first letter of the display name.
    expect(screen.getByText('A')).toBeInTheDocument();
  });

  it('falls back to the username initial when no name is set', () => {
    renderPanel({ firstName: '', lastName: '', username: 'zed' });

    expect(screen.getByText('@zed')).toBeInTheDocument();
    expect(screen.getByText('Z')).toBeInTheDocument();
  });

  it('omits the identity summary block entirely when there is no user', () => {
    renderPanel(null);

    expect(screen.queryByText('@')).not.toBeInTheDocument();
  });
});
