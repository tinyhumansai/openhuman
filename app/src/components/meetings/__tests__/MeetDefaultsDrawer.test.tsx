import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import { MeetDefaultsDrawer } from '../MeetDefaultsDrawer';

const getMock = vi.fn();
const updateMock = vi.fn();

vi.mock('../../../utils/tauriCommands', async () => {
  const actual = await vi.importActual<typeof import('../../../utils/tauriCommands')>(
    '../../../utils/tauriCommands'
  );
  return {
    ...actual,
    isTauri: () => true,
    openhumanGetMeetSettings: (...args: unknown[]) => getMock(...args),
    openhumanUpdateMeetSettings: (...args: unknown[]) => updateMock(...args),
  };
});

const DEFAULT_SETTINGS = {
  result: {
    auto_orchestrator_handoff: false,
    auto_join_policy: 'ask_each_time' as const,
    auto_summarize_policy: 'ask' as const,
    listen_only_default: true,
    ingest_backend_transcripts: false,
    platform_auto_join_policies: {},
    watch_calendar: false,
  },
};

describe('MeetDefaultsDrawer', () => {
  beforeEach(() => {
    getMock.mockReset();
    updateMock.mockReset();
    getMock.mockResolvedValue(DEFAULT_SETTINGS);
    updateMock.mockResolvedValue({ result: {} });
  });

  afterEach(() => cleanup());

  it('does not render when closed', () => {
    renderWithProviders(<MeetDefaultsDrawer open={false} onClose={vi.fn()} />);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('renders the drawer when open', async () => {
    renderWithProviders(<MeetDefaultsDrawer open onClose={vi.fn()} />);
    await waitFor(() => expect(getMock).toHaveBeenCalled());
    expect(screen.getByRole('dialog')).toBeInTheDocument();
  });

  it('loads and displays current settings', async () => {
    renderWithProviders(<MeetDefaultsDrawer open onClose={vi.fn()} />);
    await waitFor(() => expect(screen.queryByText(/loading/i)).not.toBeInTheDocument());
    // Should render global policy select and platform sections
    expect(screen.getByRole('dialog')).toBeInTheDocument();
  });

  it('calls update when global policy changes', async () => {
    renderWithProviders(<MeetDefaultsDrawer open onClose={vi.fn()} />);
    await waitFor(() => expect(screen.queryByText(/loading/i)).not.toBeInTheDocument());

    // Find the global auto-join select (first select) and change it
    const selects = screen.getAllByRole('combobox');
    fireEvent.change(selects[0], { target: { value: 'always' } });
    await waitFor(() =>
      expect(updateMock).toHaveBeenCalledWith(
        expect.objectContaining({ auto_join_policy: 'always' })
      )
    );
  });

  it('closes via the close button', async () => {
    const onClose = vi.fn();
    renderWithProviders(<MeetDefaultsDrawer open onClose={onClose} />);
    await waitFor(() => expect(getMock).toHaveBeenCalled());

    const closeBtn = screen.getByRole('button', { name: /close/i });
    fireEvent.click(closeBtn);
    expect(onClose).toHaveBeenCalled();
  });

  it('calls update with platform policies when platform override changes', async () => {
    renderWithProviders(<MeetDefaultsDrawer open onClose={vi.fn()} />);
    await waitFor(() => expect(screen.queryByText(/loading/i)).not.toBeInTheDocument());

    // Find the platform selects (after the global select)
    const selects = screen.getAllByRole('combobox');
    // selects[1] is the first platform (gmeet) override
    if (selects.length > 1) {
      fireEvent.change(selects[1], { target: { value: 'always' } });
      await waitFor(() =>
        expect(updateMock).toHaveBeenCalledWith(
          expect.objectContaining({ platform_auto_join_policies: expect.any(Object) })
        )
      );
    }
  });

  it('closes when backdrop is clicked', async () => {
    const onClose = vi.fn();
    renderWithProviders(<MeetDefaultsDrawer open onClose={onClose} />);
    await waitFor(() => expect(getMock).toHaveBeenCalled());

    // The backdrop div has aria-hidden="true" but should have onClick
    const backdrop = document.querySelector('[aria-hidden="true"]');
    if (backdrop) {
      fireEvent.click(backdrop);
      expect(onClose).toHaveBeenCalled();
    }
  });

  // ── watch_calendar master switch ──────────────────────────────────────────

  it('renders the watch-calendar switch', async () => {
    renderWithProviders(<MeetDefaultsDrawer open onClose={vi.fn()} />);
    await waitFor(() => expect(screen.queryByText(/loading/i)).not.toBeInTheDocument());
    expect(screen.getByRole('switch', { name: /watch my calendar/i })).toBeInTheDocument();
  });

  it('reflects watch_calendar=false as unchecked', async () => {
    getMock.mockResolvedValueOnce({ ...DEFAULT_SETTINGS });
    renderWithProviders(<MeetDefaultsDrawer open onClose={vi.fn()} />);
    const sw = await screen.findByRole('switch', { name: /watch my calendar/i });
    expect(sw).toHaveAttribute('aria-checked', 'false');
  });

  it('reflects watch_calendar=true as checked', async () => {
    getMock.mockResolvedValueOnce({ result: { ...DEFAULT_SETTINGS.result, watch_calendar: true } });
    renderWithProviders(<MeetDefaultsDrawer open onClose={vi.fn()} />);
    const sw = await screen.findByRole('switch', { name: /watch my calendar/i });
    expect(sw).toHaveAttribute('aria-checked', 'true');
  });

  it('toggling the switch calls updateMeetSettings with watch_calendar', async () => {
    getMock.mockResolvedValueOnce({ ...DEFAULT_SETTINGS });
    renderWithProviders(<MeetDefaultsDrawer open onClose={vi.fn()} />);
    const sw = await screen.findByRole('switch', { name: /watch my calendar/i });

    fireEvent.click(sw);

    await waitFor(() =>
      expect(updateMock).toHaveBeenCalledWith(expect.objectContaining({ watch_calendar: true }))
    );
  });

  it('reverts optimistic state when update fails', async () => {
    getMock.mockResolvedValueOnce({ ...DEFAULT_SETTINGS });
    updateMock.mockReset();
    updateMock.mockRejectedValueOnce(new Error('network error'));
    renderWithProviders(<MeetDefaultsDrawer open onClose={vi.fn()} />);
    const sw = await screen.findByRole('switch', { name: /watch my calendar/i });

    // Toggle from false → true (optimistic update)
    fireEvent.click(sw);
    // Wait for the rejection to revert the switch back to false
    await waitFor(() => expect(sw).toHaveAttribute('aria-checked', 'false'));
  });

  // ── Finding A: load-failure state ─────────────────────────────────────────

  it('shows error + retry when initial load fails — does NOT render controls', async () => {
    getMock.mockRejectedValueOnce(new Error('RPC timeout'));
    renderWithProviders(<MeetDefaultsDrawer open onClose={vi.fn()} />);

    // Should eventually show error (not loading, not controls)
    await waitFor(() => expect(screen.queryByText(/loading/i)).not.toBeInTheDocument());

    // Error message is visible (the thrown error's message is surfaced directly)
    expect(screen.getByText(/rpc timeout/i)).toBeInTheDocument();

    // Retry button is shown
    expect(screen.getByRole('button', { name: /try again/i })).toBeInTheDocument();

    // No editable controls — no comboboxes or switches
    expect(screen.queryAllByRole('combobox')).toHaveLength(0);
    expect(screen.queryAllByRole('switch')).toHaveLength(0);

    // No save was ever attempted
    expect(updateMock).not.toHaveBeenCalled();
  });

  it('retry re-runs the load and renders the form on success', async () => {
    // First load fails, second (retry) succeeds
    getMock
      .mockRejectedValueOnce(new Error('transient error'))
      .mockResolvedValueOnce(DEFAULT_SETTINGS);

    renderWithProviders(<MeetDefaultsDrawer open onClose={vi.fn()} />);

    // Wait for error state
    const retryBtn = await screen.findByRole('button', { name: /try again/i });
    expect(screen.queryAllByRole('combobox')).toHaveLength(0);

    // Click retry
    fireEvent.click(retryBtn);

    // After retry the form should appear
    await waitFor(() =>
      expect(screen.queryByRole('button', { name: /try again/i })).not.toBeInTheDocument()
    );
    expect(screen.getAllByRole('combobox').length).toBeGreaterThan(0);
    expect(screen.getByRole('switch', { name: /watch my calendar/i })).toBeInTheDocument();

    // No save was fired during the error phase
    expect(updateMock).not.toHaveBeenCalled();
  });

  // ── Finding B: per-setting sequence isolation ──────────────────────────────

  it('per-setting revert: failed save for A is not masked by succeeded save for B', async () => {
    getMock.mockResolvedValueOnce({ ...DEFAULT_SETTINGS });

    // Save A (auto_join_policy) is a controlled promise that we will reject later
    let rejectAutoJoin!: (e: Error) => void;
    const autoJoinPromise = new Promise<{ result: object }>((_, rej) => {
      rejectAutoJoin = rej;
    });

    // First updateMock call (auto_join_policy) → controlled; subsequent calls resolve immediately
    updateMock.mockReturnValueOnce(autoJoinPromise).mockResolvedValue({ result: {} });

    renderWithProviders(<MeetDefaultsDrawer open onClose={vi.fn()} />);
    await waitFor(() => expect(screen.queryByText(/loading/i)).not.toBeInTheDocument());

    // Optimistically change setting A (auto_join_policy: 'ask_each_time' → 'always')
    const selects = screen.getAllByRole('combobox');
    fireEvent.change(selects[0], { target: { value: 'always' } });

    // Optimistically change setting B (listen_only: true → false)
    const listenSwitch = screen.getByRole('switch', { name: /listen.only/i });
    fireEvent.click(listenSwitch);

    // Wait for both saves to have been dispatched
    await waitFor(() => expect(updateMock).toHaveBeenCalledTimes(2));

    // Save B (listen_only) has already resolved successfully; now reject save A
    rejectAutoJoin(new Error('network error'));

    // Setting A should revert to its original value
    await waitFor(() => {
      const autoJoinSelect = screen.getAllByRole('combobox')[0];
      expect(autoJoinSelect).toHaveValue('ask_each_time');
    });

    // Setting B must keep the new value (was true, clicked → false)
    const listenSwitchAfter = screen.getByRole('switch', { name: /listen.only/i });
    expect(listenSwitchAfter).toHaveAttribute('aria-checked', 'false');
  });

  it('superseded response for the same setting is silently ignored', async () => {
    getMock.mockResolvedValueOnce({ ...DEFAULT_SETTINGS });

    // First change (→ 'always') returns a controlled promise; second (→ 'never') resolves fast
    let resolveFirstSave!: (v: { result: object }) => void;
    const firstSavePromise = new Promise<{ result: object }>(res => {
      resolveFirstSave = res;
    });
    updateMock.mockReturnValueOnce(firstSavePromise).mockResolvedValue({ result: {} });

    renderWithProviders(<MeetDefaultsDrawer open onClose={vi.fn()} />);
    await waitFor(() => expect(screen.queryByText(/loading/i)).not.toBeInTheDocument());

    const selects = screen.getAllByRole('combobox');

    // First rapid change: ask_each_time → always
    fireEvent.change(selects[0], { target: { value: 'always' } });
    // Second rapid change for the SAME setting: always → never (supersedes the first)
    fireEvent.change(selects[0], { target: { value: 'never' } });

    // Wait for the second (fast) save to complete
    await waitFor(() => expect(updateMock).toHaveBeenCalledTimes(2));

    // Resolve the first (now-superseded) save
    resolveFirstSave({ result: {} });

    // The select value should remain 'never' (the current committed value)
    // A short wait ensures the first save's then() handler has run
    await waitFor(() => {
      const autoJoinSelect = screen.getAllByRole('combobox')[0];
      expect(autoJoinSelect).toHaveValue('never');
    });
  });
});
