import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  type MeetAgentEvent,
  meetAgentJoin,
  meetAgentLeave,
  subscribeMeetAgentEvents,
} from './meetAgent';
import { isTauri } from './webviewAccountService';

// Capture listeners registered via `listen(...)`.
type EventHandler = (evt: { payload: unknown }) => void;
const listeners = new Map<string, EventHandler>();

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
  isTauri: vi.fn().mockReturnValue(true),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, handler: EventHandler) => {
    listeners.set(event, handler);
    return () => {
      listeners.delete(event);
    };
  }),
}));

// webviewAccountService re-exports isTauri — mock the same underlying module.
vi.mock('./webviewAccountService', () => ({ isTauri: vi.fn().mockReturnValue(true) }));

const mockInvoke = invoke as ReturnType<typeof vi.fn>;
const mockIsTauri = isTauri as ReturnType<typeof vi.fn>;

function fireWebviewEvent(payload: {
  account_id: string;
  provider: string;
  kind: string;
  payload: Record<string, unknown>;
}): void {
  const handler = listeners.get('webview:event');
  if (!handler) throw new Error('webview:event listener not attached');
  handler({ payload });
}

beforeEach(() => {
  listeners.clear();
  mockInvoke.mockResolvedValue(undefined);
  mockIsTauri.mockReturnValue(true);
});

// ─── meetAgentJoin ─────────────────────────────────────────────────────────

describe('meetAgentJoin', () => {
  it('invokes webview_meet_agent_join with correct args', async () => {
    await meetAgentJoin({ accountId: 'acc-1', meetingUrl: 'https://meet.google.com/abc-defg-hij' });
    expect(mockInvoke).toHaveBeenCalledWith('webview_meet_agent_join', {
      args: { accountId: 'acc-1', meetingUrl: 'https://meet.google.com/abc-defg-hij' },
    });
  });

  it('no-ops when not in Tauri', async () => {
    mockIsTauri.mockReturnValue(false);
    await meetAgentJoin({ accountId: 'acc-1', meetingUrl: 'https://meet.google.com/abc-defg-hij' });
    expect(mockInvoke).not.toHaveBeenCalled();
  });

  it('propagates invoke errors', async () => {
    mockInvoke.mockRejectedValue(new Error('tauri error'));
    await expect(
      meetAgentJoin({ accountId: 'acc-1', meetingUrl: 'https://meet.google.com/abc-defg-hij' })
    ).rejects.toThrow('tauri error');
  });
});

// ─── meetAgentLeave ────────────────────────────────────────────────────────

describe('meetAgentLeave', () => {
  it('invokes webview_meet_agent_leave with correct args', async () => {
    await meetAgentLeave({ accountId: 'acc-1' });
    expect(mockInvoke).toHaveBeenCalledWith('webview_meet_agent_leave', {
      args: { accountId: 'acc-1' },
    });
  });

  it('no-ops when not in Tauri', async () => {
    mockIsTauri.mockReturnValue(false);
    await meetAgentLeave({ accountId: 'acc-1' });
    expect(mockInvoke).not.toHaveBeenCalled();
  });
});

// ─── subscribeMeetAgentEvents ──────────────────────────────────────────────

describe('subscribeMeetAgentEvents', () => {
  it('delivers meet_agent_joined event', async () => {
    const events: MeetAgentEvent[] = [];
    subscribeMeetAgentEvents(e => events.push(e));

    // Allow the listen promise to resolve.
    await Promise.resolve();
    await Promise.resolve();

    fireWebviewEvent({
      account_id: 'acc-1',
      provider: 'google-meet',
      kind: 'meet_agent_joined',
      payload: { code: 'abc-defg-hij', joinedAt: 1700000000000 },
    });

    expect(events).toHaveLength(1);
    expect(events[0]).toEqual({
      kind: 'meet_agent_joined',
      accountId: 'acc-1',
      code: 'abc-defg-hij',
      joinedAt: 1700000000000,
    });
  });

  it('delivers meet_agent_left event', async () => {
    const events: MeetAgentEvent[] = [];
    subscribeMeetAgentEvents(e => events.push(e));
    await Promise.resolve();
    await Promise.resolve();

    fireWebviewEvent({
      account_id: 'acc-1',
      provider: 'google-meet',
      kind: 'meet_agent_left',
      payload: { reason: 'leave-button-gone' },
    });

    expect(events).toHaveLength(1);
    expect(events[0]).toEqual({
      kind: 'meet_agent_left',
      accountId: 'acc-1',
      reason: 'leave-button-gone',
    });
  });

  it('delivers meet_agent_failed event', async () => {
    const events: MeetAgentEvent[] = [];
    subscribeMeetAgentEvents(e => events.push(e));
    await Promise.resolve();
    await Promise.resolve();

    fireWebviewEvent({
      account_id: 'acc-1',
      provider: 'google-meet',
      kind: 'meet_agent_failed',
      payload: { reason: 'timeout' },
    });

    expect(events).toHaveLength(1);
    expect(events[0]).toEqual({ kind: 'meet_agent_failed', accountId: 'acc-1', reason: 'timeout' });
  });

  it('filters out non-agent events', async () => {
    const events: MeetAgentEvent[] = [];
    subscribeMeetAgentEvents(e => events.push(e));
    await Promise.resolve();
    await Promise.resolve();

    fireWebviewEvent({
      account_id: 'acc-1',
      provider: 'google-meet',
      kind: 'meet_call_started',
      payload: { code: 'abc-defg-hij' },
    });

    expect(events).toHaveLength(0);
  });

  it('returns a working unsubscribe function', async () => {
    const events: MeetAgentEvent[] = [];
    const unsub = subscribeMeetAgentEvents(e => events.push(e));
    await Promise.resolve();
    await Promise.resolve();

    unsub();

    // After unsubscribe the listener should have been removed.
    expect(listeners.has('webview:event')).toBe(false);
  });

  it('no-ops when not in Tauri and returns empty unsubscribe', async () => {
    mockIsTauri.mockReturnValue(false);
    const events: MeetAgentEvent[] = [];
    const unsub = subscribeMeetAgentEvents(e => events.push(e));
    // Should not have tried to listen.
    expect(listeners.has('webview:event')).toBe(false);
    expect(typeof unsub).toBe('function');
    unsub(); // should not throw
    expect(events).toHaveLength(0);
  });
});
