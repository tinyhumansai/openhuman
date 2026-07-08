import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import MeetingsPage from '../MeetingsPage';

// ---------------------------------------------------------------------------
// Mock the Tauri bridge so the page believes it's running in Tauri and we
// control the meet-settings payload it loads on mount / after the drawer closes.
// ---------------------------------------------------------------------------

const getMeetSettingsMock = vi.fn();

vi.mock('../../../utils/tauriCommands', () => ({
  isTauri: () => true,
  openhumanGetMeetSettings: (...args: unknown[]) => getMeetSettingsMock(...args),
}));

// ---------------------------------------------------------------------------
// Stub the heavy children so the test stays focused on MeetingsPage's own
// settings-loading + drawer-close re-fetch wiring.
// ---------------------------------------------------------------------------

// UpcomingTable is the consumer we assert on — surface the replyDisplayName prop.
vi.mock('../UpcomingTable', () => ({
  UpcomingTable: ({ replyDisplayName }: { replyDisplayName: string }) => (
    <div data-testid="upcoming-table" data-reply-name={replyDisplayName} />
  ),
}));

// The remaining children carry no behavior relevant here — trivial stubs.
vi.mock('../../recallCalendar/RecallCalendarCard', () => ({
  default: () => <div data-testid="recall-calendar-card-stub" />,
}));
vi.mock('../MeetComposer', () => ({
  MeetComposer: () => <div data-testid="meet-composer-stub" />,
}));
vi.mock('../ActiveMeetingBanner', () => ({
  ActiveMeetingBanner: () => <div data-testid="active-meeting-banner-stub" />,
}));
vi.mock('../HistorySection', () => ({ default: () => <div data-testid="history-section-stub" /> }));

// Lightweight drawer stub: renders a Close button (which calls onClose) only
// while open, so the drawer-close re-fetch path in MeetingsPage can be driven
// without pulling in the real drawer's own settings load.
vi.mock('../MeetDefaultsDrawer', () => ({
  MeetDefaultsDrawer: ({ open, onClose }: { open: boolean; onClose: () => void }) =>
    open ? (
      <button type="button" data-testid="drawer-close-stub" onClick={onClose}>
        close-drawer
      </button>
    ) : null,
}));

function mockSettings(reply_display_name: string, watch_calendar = true) {
  getMeetSettingsMock.mockResolvedValue({ result: { watch_calendar, reply_display_name } });
}

describe('MeetingsPage', () => {
  beforeEach(() => {
    getMeetSettingsMock.mockReset();
    mockSettings('Alex Kim');
  });

  afterEach(() => cleanup());

  it('loads reply_display_name and passes it to UpcomingTable', async () => {
    renderWithProviders(<MeetingsPage />);

    await waitFor(() =>
      expect(screen.getByTestId('upcoming-table')).toHaveAttribute('data-reply-name', 'Alex Kim')
    );
    expect(getMeetSettingsMock).toHaveBeenCalledOnce();
  });

  it('refreshes settings after the meeting-defaults drawer closes', async () => {
    renderWithProviders(<MeetingsPage />);

    // On-mount fetch settles first.
    await waitFor(() =>
      expect(screen.getByTestId('upcoming-table')).toHaveAttribute('data-reply-name', 'Alex Kim')
    );
    expect(getMeetSettingsMock).toHaveBeenCalledOnce();

    // The gear button opens the drawer (aria-label from i18n → "Meeting defaults").
    fireEvent.click(screen.getByRole('button', { name: /meeting defaults/i }));

    // A subsequent load returns an updated display name so we can prove the
    // re-fetch actually re-threads the value into UpcomingTable.
    mockSettings('Sam Rivers');
    fireEvent.click(screen.getByTestId('drawer-close-stub'));

    await waitFor(() => expect(getMeetSettingsMock).toHaveBeenCalledTimes(2));
    await waitFor(() =>
      expect(screen.getByTestId('upcoming-table')).toHaveAttribute('data-reply-name', 'Sam Rivers')
    );
  });
});
