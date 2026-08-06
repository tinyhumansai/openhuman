import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  COMPANION_STATE_CHANGED_EVENT,
  parseCompanionStateChangedEvent,
  subscribeCompanionStateChanged,
} from '../companionEvents';

const mocks = vi.hoisted(() => ({ dispatch: vi.fn(), isTauri: vi.fn(), listen: vi.fn() }));

vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));
vi.mock('../../store/index', () => ({ store: { dispatch: mocks.dispatch } }));
vi.mock('../../utils/tauriCommands/common', () => ({ isTauri: mocks.isTauri }));

// The companion state change now arrives as a Tauri `companion://state_changed`
// event with a camelCase payload `{ sessionId, state, previousState }`.
describe('parseCompanionStateChangedEvent', () => {
  it('returns null for non-object inputs', () => {
    expect(parseCompanionStateChangedEvent(null)).toBeNull();
    expect(parseCompanionStateChangedEvent(undefined)).toBeNull();
    expect(parseCompanionStateChangedEvent(42)).toBeNull();
    expect(parseCompanionStateChangedEvent('listening')).toBeNull();
  });

  it('returns null when sessionId is missing or non-string', () => {
    expect(parseCompanionStateChangedEvent({ state: 'listening' })).toBeNull();
    expect(parseCompanionStateChangedEvent({ sessionId: 42, state: 'listening' })).toBeNull();
  });

  it('returns null when state is missing or not in the enum', () => {
    expect(parseCompanionStateChangedEvent({ sessionId: 's1' })).toBeNull();
    expect(parseCompanionStateChangedEvent({ sessionId: 's1', state: 'unknown' })).toBeNull();
    expect(parseCompanionStateChangedEvent({ sessionId: 's1', state: 7 })).toBeNull();
  });

  it('accepts a valid payload and round-trips all fields', () => {
    const event = parseCompanionStateChangedEvent({
      sessionId: 'sess-1',
      state: 'speaking',
      previousState: 'thinking',
    });
    expect(event).toEqual({ sessionId: 'sess-1', state: 'speaking', previousState: 'thinking' });
  });

  it("defaults previousState to 'idle' when missing or invalid", () => {
    const missing = parseCompanionStateChangedEvent({ sessionId: 's', state: 'listening' });
    expect(missing?.previousState).toBe('idle');

    const invalid = parseCompanionStateChangedEvent({
      sessionId: 's',
      state: 'listening',
      previousState: 'banana',
    });
    expect(invalid?.previousState).toBe('idle');
  });

  it('accepts every valid state value', () => {
    for (const state of ['idle', 'listening', 'thinking', 'speaking', 'error']) {
      const event = parseCompanionStateChangedEvent({ sessionId: 's', state });
      expect(event?.state).toBe(state);
    }
  });
});

describe('subscribeCompanionStateChanged', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.isTauri.mockReturnValue(true);
  });

  it('drops invalid shell events and dispatches valid state changes', async () => {
    let handler: ((event: { payload: unknown }) => void) | undefined;
    const unlisten = vi.fn();
    mocks.listen.mockImplementation(
      async (_eventName: string, listener: (event: { payload: unknown }) => void) => {
        handler = listener;
        return unlisten;
      }
    );
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined);

    await expect(subscribeCompanionStateChanged()).resolves.toBe(unlisten);
    expect(mocks.listen).toHaveBeenCalledWith(COMPANION_STATE_CHANGED_EVENT, expect.any(Function));

    handler?.({ payload: { sessionId: 'session-1', state: 'invalid' } });
    expect(warn).toHaveBeenCalledWith('[companion] state_changed dropped — invalid payload shape');
    expect(mocks.dispatch).not.toHaveBeenCalled();

    handler?.({
      payload: { sessionId: 'session-1', state: 'speaking', previousState: 'thinking' },
    });
    expect(mocks.dispatch).toHaveBeenCalledWith({
      type: 'companion/setCompanionState',
      payload: { sessionId: 'session-1', state: 'speaking', previousState: 'thinking' },
    });
  });
});
