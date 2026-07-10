import { render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { Announcement } from '../../services/announcementService';
import { markAnnouncementShown } from '../../store/announcementSlice';
import AnnouncementGate from './AnnouncementGate';

// Controllable mock state shared across the mocked modules.
const authState: { isAuthenticated: boolean; userId: string | null } = {
  isAuthenticated: true,
  userId: 'u1',
};
let shownIds: string[] = [];
const dispatch = vi.fn();
const fetchLatestAnnouncement = vi.fn();

vi.mock('../../providers/CoreStateProvider', () => ({
  useCoreState: () => ({ snapshot: { auth: authState } }),
}));

vi.mock('../../services/announcementService', () => ({
  fetchLatestAnnouncement: () => fetchLatestAnnouncement(),
}));

vi.mock('../../store/hooks', () => ({
  useAppDispatch: () => dispatch,
  // Run the real selector against our controllable state.
  useAppSelector: (sel: (s: unknown) => unknown) => sel({ announcement: { shownIds } }),
}));

// Stub the modal so this suite tests gate logic, not presentation.
vi.mock('./AnnouncementModal', () => ({
  default: ({ announcement, onDismiss }: { announcement: Announcement; onDismiss: () => void }) => (
    <div data-testid="modal">
      <span>{announcement.title}</span>
      <button type="button" data-testid="dismiss" onClick={onDismiss}>
        dismiss
      </button>
    </div>
  ),
}));

const sample: Announcement = {
  id: 'a1',
  title: 'Hello',
  body: 'World',
  severity: 'INFO',
  cta: null,
  startsAt: null,
  expiresAt: null,
  createdAt: null,
};

describe('AnnouncementGate', () => {
  beforeEach(() => {
    authState.isAuthenticated = true;
    authState.userId = 'u1';
    shownIds = [];
    dispatch.mockClear();
    fetchLatestAnnouncement.mockReset();
  });

  afterEach(() => vi.clearAllMocks());

  it('shows the announcement once fetched when authenticated and unseen', async () => {
    fetchLatestAnnouncement.mockResolvedValue(sample);
    render(<AnnouncementGate />);
    expect(await screen.findByTestId('modal')).toBeInTheDocument();
    expect(screen.getByText('Hello')).toBeInTheDocument();
  });

  it('does not fetch or render when unauthenticated', async () => {
    authState.isAuthenticated = false;
    render(<AnnouncementGate />);
    await waitFor(() => expect(fetchLatestAnnouncement).not.toHaveBeenCalled());
    expect(screen.queryByTestId('modal')).not.toBeInTheDocument();
  });

  it('renders nothing when the fetched announcement was already shown', async () => {
    shownIds = ['a1'];
    fetchLatestAnnouncement.mockResolvedValue(sample);
    render(<AnnouncementGate />);
    await waitFor(() => expect(fetchLatestAnnouncement).toHaveBeenCalled());
    expect(screen.queryByTestId('modal')).not.toBeInTheDocument();
  });

  it('renders nothing when there is no active announcement', async () => {
    fetchLatestAnnouncement.mockResolvedValue(null);
    render(<AnnouncementGate />);
    await waitFor(() => expect(fetchLatestAnnouncement).toHaveBeenCalled());
    expect(screen.queryByTestId('modal')).not.toBeInTheDocument();
  });

  it('records the announcement as shown and hides it on dismiss', async () => {
    fetchLatestAnnouncement.mockResolvedValue(sample);
    render(<AnnouncementGate />);
    const dismiss = await screen.findByTestId('dismiss');
    dismiss.click();
    expect(dispatch).toHaveBeenCalledWith(markAnnouncementShown('a1'));
    await waitFor(() => expect(screen.queryByTestId('modal')).not.toBeInTheDocument());
  });

  it('swallows a fetch error without rendering', async () => {
    fetchLatestAnnouncement.mockRejectedValue(new Error('boom'));
    render(<AnnouncementGate />);
    await waitFor(() => expect(fetchLatestAnnouncement).toHaveBeenCalled());
    expect(screen.queryByTestId('modal')).not.toBeInTheDocument();
  });
});
