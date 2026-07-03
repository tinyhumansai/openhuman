import { cleanup, fireEvent, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import RecallCalendarCard from '../RecallCalendarCard';

// ---------------------------------------------------------------------------
// Mock the connection hook so we drive card state directly. The card is a pure
// presenter over useRecallCalendar; all network/poll behavior lives in the hook.
// ---------------------------------------------------------------------------

const beginConnect = vi.fn();
const disconnect = vi.fn();
const refresh = vi.fn();
const useRecallCalendarMock = vi.fn();

vi.mock('../../../lib/recallCalendar/hooks', () => ({
  useRecallCalendar: (...args: unknown[]) => useRecallCalendarMock(...args),
}));

interface HookState {
  status: { enabled: boolean; connected: boolean; email?: string } | null;
  loading: boolean;
  busy: boolean;
  error: string | null;
}

function setHook(overrides: Partial<HookState> = {}) {
  useRecallCalendarMock.mockReturnValue({
    status: { enabled: true, connected: false },
    loading: false,
    busy: false,
    error: null,
    beginConnect,
    disconnect,
    refresh,
    ...overrides,
  });
}

describe('RecallCalendarCard', () => {
  beforeEach(() => {
    beginConnect.mockReset().mockResolvedValue(undefined);
    disconnect.mockReset().mockResolvedValue(undefined);
    refresh.mockReset().mockResolvedValue(undefined);
    useRecallCalendarMock.mockReset();
    setHook();
  });

  afterEach(() => cleanup());

  it('renders nothing while the hook is loading', () => {
    setHook({ loading: true });
    renderWithProviders(<RecallCalendarCard />);
    expect(screen.queryByTestId('recall-calendar-card')).not.toBeInTheDocument();
  });

  it('renders nothing when the integration is disabled', () => {
    setHook({ status: { enabled: false, connected: false } });
    renderWithProviders(<RecallCalendarCard />);
    expect(screen.queryByTestId('recall-calendar-card')).not.toBeInTheDocument();
  });

  it('renders nothing when the status is null', () => {
    setHook({ status: null });
    renderWithProviders(<RecallCalendarCard />);
    expect(screen.queryByTestId('recall-calendar-card')).not.toBeInTheDocument();
  });

  it('shows the connect button when enabled but not connected, and calls beginConnect on click', () => {
    setHook({ status: { enabled: true, connected: false } });
    renderWithProviders(<RecallCalendarCard />);

    expect(screen.getByTestId('recall-calendar-card')).toBeInTheDocument();
    // Description (not-connected state) is shown.
    expect(screen.getByText(/auto-join google meet/i)).toBeInTheDocument();
    // No disconnect action in the not-connected state.
    expect(screen.queryByTestId('recall-calendar-disconnect')).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId('recall-calendar-connect'));
    expect(beginConnect).toHaveBeenCalledOnce();
  });

  it('shows the connected email + disconnect button and calls disconnect on click', () => {
    setHook({ status: { enabled: true, connected: true, email: 'me@example.com' } });
    renderWithProviders(<RecallCalendarCard />);

    expect(screen.getByText('me@example.com')).toBeInTheDocument();
    expect(screen.queryByTestId('recall-calendar-connect')).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId('recall-calendar-disconnect'));
    expect(disconnect).toHaveBeenCalledOnce();
  });

  it('falls back to the connected label when connected without an email', () => {
    setHook({ status: { enabled: true, connected: true } });
    renderWithProviders(<RecallCalendarCard />);
    expect(screen.getByText('Connected')).toBeInTheDocument();
    expect(screen.getByTestId('recall-calendar-disconnect')).toBeInTheDocument();
  });

  it('renders the error string when the hook reports an error', () => {
    setHook({ status: { enabled: true, connected: false }, error: 'Something broke' });
    renderWithProviders(<RecallCalendarCard />);
    expect(screen.getByText('Something broke')).toBeInTheDocument();
  });

  it('disables the connect button and shows a spinner while busy', () => {
    setHook({ status: { enabled: true, connected: false }, busy: true });
    renderWithProviders(<RecallCalendarCard />);
    expect(screen.getByTestId('recall-calendar-connect')).toBeDisabled();
  });

  it('disables the disconnect button while busy', () => {
    setHook({ status: { enabled: true, connected: true, email: 'me@example.com' }, busy: true });
    renderWithProviders(<RecallCalendarCard />);
    expect(screen.getByTestId('recall-calendar-disconnect')).toBeDisabled();
  });
});
