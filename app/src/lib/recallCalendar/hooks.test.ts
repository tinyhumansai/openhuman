import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

import * as recallCalendarApi from './recallCalendarApi';
import { openhumanUpdateMeetSettings } from '../../utils/tauriCommands/config';
import { useRecallCalendar } from './hooks';

vi.mock('../../utils/tauriCommands/config', () => ({ openhumanUpdateMeetSettings: vi.fn() }));

vi.mock('../../utils/openUrl', () => ({ openUrl: vi.fn() }));

vi.mock('./recallCalendarApi', () => ({ status: vi.fn(), connect: vi.fn(), disconnect: vi.fn() }));

describe('useRecallCalendar', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(openhumanUpdateMeetSettings).mockResolvedValue({
      result: { config: {}, workspace_dir: '/tmp', config_path: '/tmp/config.toml' },
      logs: [],
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  test('syncs Recall as provider when initial status is already connected', async () => {
    vi.mocked(recallCalendarApi.status).mockResolvedValue({
      enabled: true,
      connected: true,
      email: 'user@example.com',
    });

    renderHook(() => useRecallCalendar());

    await waitFor(() => {
      expect(openhumanUpdateMeetSettings).toHaveBeenCalledWith({ calendar_provider: 'recall' });
    });
  });

  test('clears a stale generic error once a later status poll succeeds', async () => {
    // Benign default: not connected → provider settles on 'composio'.
    vi.mocked(recallCalendarApi.status).mockResolvedValue({ enabled: false, connected: false });

    const { result } = renderHook(() => useRecallCalendar());

    // Mount poll syncs the provider once; no error.
    await waitFor(() =>
      expect(openhumanUpdateMeetSettings).toHaveBeenCalledWith({ calendar_provider: 'composio' })
    );
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.error).toBeNull();

    // A transient fetch failure records a generic error.
    vi.mocked(recallCalendarApi.status).mockRejectedValueOnce(new Error('network blip'));
    await act(async () => {
      await result.current.refresh();
    });
    expect(result.current.error).toContain('network blip');

    // A later successful poll — provider already 'composio', so the flip branch
    // is skipped — must still clear the stale error (regression guard).
    await act(async () => {
      await result.current.refresh();
    });
    await waitFor(() => expect(result.current.error).toBeNull());
  });

  test('beginConnect surfaces an error when connect() rejects', async () => {
    vi.mocked(recallCalendarApi.status).mockResolvedValue({ enabled: false, connected: false });
    vi.mocked(recallCalendarApi.connect).mockRejectedValue(new Error('connect boom'));

    const { result } = renderHook(() => useRecallCalendar());
    await waitFor(() => expect(result.current.loading).toBe(false));

    await act(async () => {
      await result.current.beginConnect();
    });

    expect(result.current.error).toContain('connect boom');
    expect(result.current.busy).toBe(false);
  });

  test('disconnect surfaces an error when disconnect() rejects', async () => {
    vi.mocked(recallCalendarApi.status).mockResolvedValue({ enabled: false, connected: false });
    vi.mocked(recallCalendarApi.disconnect).mockRejectedValue(new Error('disconnect boom'));

    const { result } = renderHook(() => useRecallCalendar());
    await waitFor(() => expect(result.current.loading).toBe(false));

    await act(async () => {
      await result.current.disconnect();
    });

    expect(result.current.error).toContain('disconnect boom');
    expect(result.current.busy).toBe(false);
  });

  test('records a provider-switch error when the config flip fails', async () => {
    vi.mocked(recallCalendarApi.status).mockResolvedValue({
      enabled: true,
      connected: true,
      email: 'user@example.com',
    });
    vi.mocked(openhumanUpdateMeetSettings).mockRejectedValue(new Error('flip boom'));

    const { result } = renderHook(() => useRecallCalendar());

    await waitFor(() => expect(result.current.error).toMatch(/^calendar provider switch failed:/));
  });
});
