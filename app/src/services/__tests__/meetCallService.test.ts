import { invoke, isTauri } from '@tauri-apps/api/core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../coreRpcClient';
import { closeMeetCall, joinMeetCall } from '../meetCallService';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: vi.fn() }));

vi.mock('../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

describe('joinMeetCall', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(isTauri).mockReturnValue(true);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('rejects empty inputs without contacting the core', async () => {
    await expect(joinMeetCall({ meetUrl: '   ', displayName: 'Alice' })).rejects.toThrow(
      /Meet link/i
    );
    await expect(
      joinMeetCall({ meetUrl: 'https://meet.google.com/abc-defg-hij', displayName: '' })
    ).rejects.toThrow(/display name/i);
    expect(callCoreRpc).not.toHaveBeenCalled();
    expect(invoke).not.toHaveBeenCalled();
  });

  it('chains the core RPC into the Tauri window-open command', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      ok: true,
      request_id: 'req-1',
      meet_url: 'https://meet.google.com/abc-defg-hij',
      display_name: 'Agent Alice',
    } as never);
    vi.mocked(invoke).mockResolvedValueOnce('meet-call-req-1');

    const result = await joinMeetCall({
      meetUrl: 'https://meet.google.com/abc-defg-hij',
      displayName: 'Agent Alice',
    });

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.meet_join_call',
      params: { meet_url: 'https://meet.google.com/abc-defg-hij', display_name: 'Agent Alice' },
    });
    expect(invoke).toHaveBeenCalledWith('meet_call_open_window', {
      args: {
        request_id: 'req-1',
        meet_url: 'https://meet.google.com/abc-defg-hij',
        display_name: 'Agent Alice',
      },
    });
    expect(result).toEqual({
      requestId: 'req-1',
      meetUrl: 'https://meet.google.com/abc-defg-hij',
      displayName: 'Agent Alice',
      windowLabel: 'meet-call-req-1',
    });
  });

  it('throws if core rejects the request', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ ok: false } as never);
    await expect(
      joinMeetCall({ meetUrl: 'https://meet.google.com/abc-defg-hij', displayName: 'Agent Alice' })
    ).rejects.toThrow(/Core rejected/);
    expect(invoke).not.toHaveBeenCalled();
  });

  it('refuses to open a window outside the desktop shell', async () => {
    vi.mocked(isTauri).mockReturnValue(false);
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      ok: true,
      request_id: 'req-1',
      meet_url: 'https://meet.google.com/abc-defg-hij',
      display_name: 'Agent Alice',
    } as never);

    await expect(
      joinMeetCall({ meetUrl: 'https://meet.google.com/abc-defg-hij', displayName: 'Agent Alice' })
    ).rejects.toThrow(/desktop app/);
    expect(invoke).not.toHaveBeenCalled();
  });
});

describe('closeMeetCall', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('forwards the request_id and returns the shell result', async () => {
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(invoke).mockResolvedValueOnce(true);

    await expect(closeMeetCall('req-1')).resolves.toBe(true);
    expect(invoke).toHaveBeenCalledWith('meet_call_close_window', { requestId: 'req-1' });
  });

  it('is a no-op outside the desktop shell', async () => {
    vi.mocked(isTauri).mockReturnValue(false);

    await expect(closeMeetCall('req-1')).resolves.toBe(false);
    expect(invoke).not.toHaveBeenCalled();
  });
});
