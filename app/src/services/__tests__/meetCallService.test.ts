import { invoke, isTauri } from '@tauri-apps/api/core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../coreRpcClient';
import { closeMeetCall, joinMeetCall, listMeetCalls } from '../meetCallService';

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
      ownerDisplayName: 'Owner Bob',
    });

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.meet_join_call',
      params: { meet_url: 'https://meet.google.com/abc-defg-hij', display_name: 'Agent Alice' },
    });
    // owner_display_name is forwarded to the shell (not to the core's
    // meet_join_call, which is stateless validation only) — assert on
    // the shell args, not the core RPC params.
    expect(invoke).toHaveBeenCalledWith('meet_call_open_window', {
      args: {
        request_id: 'req-1',
        meet_url: 'https://meet.google.com/abc-defg-hij',
        display_name: 'Agent Alice',
        owner_display_name: 'Owner Bob',
      },
    });
    expect(result).toEqual({
      requestId: 'req-1',
      meetUrl: 'https://meet.google.com/abc-defg-hij',
      displayName: 'Agent Alice',
      ownerDisplayName: 'Owner Bob',
      windowLabel: 'meet-call-req-1',
    });
  });

  it('throws if core rejects the request', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ ok: false } as never);
    await expect(
      joinMeetCall({
        meetUrl: 'https://meet.google.com/abc-defg-hij',
        displayName: 'Agent Alice',
        ownerDisplayName: 'Owner Bob',
      })
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
      joinMeetCall({
        meetUrl: 'https://meet.google.com/abc-defg-hij',
        displayName: 'Agent Alice',
        ownerDisplayName: 'Owner Bob',
      })
    ).rejects.toThrow(/desktop app/);
    expect(invoke).not.toHaveBeenCalled();
  });

  it('rejects an empty owner_display_name as a privacy-lock guard', async () => {
    // Privacy lock: empty owner would fail closed at the core wake
    // gate (no captions ever wake the bot). Surface the requirement
    // up front so the user doesn't sit through a join only to find
    // the bot silent — see feat/mascot-meet-flowA Plan C.
    await expect(
      joinMeetCall({
        meetUrl: 'https://meet.google.com/abc-defg-hij',
        displayName: 'Agent Alice',
        ownerDisplayName: '   ',
      })
    ).rejects.toThrow(/your own name/i);
    expect(callCoreRpc).not.toHaveBeenCalled();
    expect(invoke).not.toHaveBeenCalled();
  });
});

describe('listMeetCalls', () => {
  beforeEach(() => {
    // Use mockReset (not just clearAllMocks) to drain any once-queues
    // left over from the joinMeetCall describe block above, ensuring
    // each test below starts with a fresh callCoreRpc mock.
    vi.mocked(callCoreRpc).mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('returns the calls array from a successful core response', async () => {
    const mockCalls = [
      {
        request_id: 'req-1',
        meet_url: 'https://meet.google.com/abc-defg-hij',
        bot_display_name: 'OpenHuman',
        owner_display_name: 'Alice',
        started_at_ms: 1700000000000,
        ended_at_ms: 1700000060000,
        listened_seconds: 30,
        spoken_seconds: 30,
        turn_count: 3,
      },
    ];
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ ok: true, calls: mockCalls, count: 1 } as never);

    const result = await listMeetCalls(20);

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.meet_agent_list_calls',
      params: { limit: 20 },
    });
    expect(result).toEqual(mockCalls);
  });

  it('returns an empty array when the core response has no calls field', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ ok: true, calls: undefined, count: 0 } as never);

    const result = await listMeetCalls(10);

    expect(result).toEqual([]);
  });

  it('throws when the core responds with ok: false', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ ok: false } as never);

    await expect(listMeetCalls(20)).rejects.toThrow(/meet_agent_list_calls/);
  });

  it('throws when the core responds with a falsy result', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce(null as never);

    await expect(listMeetCalls(20)).rejects.toThrow(/meet_agent_list_calls/);
  });

  it('uses the default limit of 20 when no argument is provided', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ ok: true, calls: [], count: 0 } as never);

    await listMeetCalls();

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.meet_agent_list_calls',
      params: { limit: 20 },
    });
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
