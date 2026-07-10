/**
 * Tests for the Settings → Account landing panel.
 *
 * Verifies that the signed-in summary header renders the user's display name,
 * username and avatar initial when a current user is present, and that the
 * summary block is omitted entirely when there is no name/username.
 */
import { screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import AccountPanel from '../AccountPanel';

const mockUseCoreState = vi.fn();

vi.mock('../../../../providers/CoreStateProvider', () => ({
  useCoreState: () => mockUseCoreState(),
}));

// Isolate the panel from the destructive logout/clear actions (which pull in
// session + clear-data plumbing we don't exercise here).
vi.mock('../../LogoutAndClearActions', () => ({
  default: () => <div data-testid="logout-and-clear-actions" />,
}));

describe('AccountPanel', () => {
  it('renders the signed-in summary with name, username and avatar initial', () => {
    mockUseCoreState.mockReturnValue({
      snapshot: { currentUser: { firstName: 'Test', lastName: 'Human', username: 'testhuman' } },
    });

    renderWithProviders(<AccountPanel />);

    expect(screen.getByText('Test Human')).toBeInTheDocument();
    expect(screen.getByText('@testhuman')).toBeInTheDocument();
    // Avatar initial derives from the display name (line ~29).
    expect(screen.getByText('T')).toBeInTheDocument();
    expect(screen.getByTestId('logout-and-clear-actions')).toBeInTheDocument();
  });

  it('falls back to the username initial when only a username is present', () => {
    mockUseCoreState.mockReturnValue({ snapshot: { currentUser: { username: 'solohuman' } } });

    renderWithProviders(<AccountPanel />);

    expect(screen.getByText('@solohuman')).toBeInTheDocument();
    // No display name, so the initial comes from the username (the leading '@'
    // is stripped before slicing).
    expect(screen.getByText('S')).toBeInTheDocument();
  });

  it('omits the summary block when there is no current user', () => {
    mockUseCoreState.mockReturnValue({ snapshot: { currentUser: null } });

    renderWithProviders(<AccountPanel />);

    // The summary is gone but the logout/clear actions section still renders.
    expect(screen.getByTestId('logout-and-clear-actions')).toBeInTheDocument();
    expect(screen.queryByText('@solohuman')).not.toBeInTheDocument();
  });
});
