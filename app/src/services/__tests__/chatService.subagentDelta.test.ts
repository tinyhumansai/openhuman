import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  type ChatSubagentTextDeltaEvent,
  type ChatSubagentThinkingDeltaEvent,
  subscribeChatEvents,
} from '../chatService';
import { socketService } from '../socketService';

/**
 * A minimal fake socket.io client capturing `.on` registrations so a test can
 * fire a given event by name and assert the wired callback runs.
 */
function fakeSocket() {
  const handlers = new Map<string, (payload: unknown) => void>();
  return {
    on: vi.fn((event: string, cb: (payload: unknown) => void) => {
      handlers.set(event, cb);
    }),
    off: vi.fn((event: string) => {
      handlers.delete(event);
    }),
    emit: (event: string, payload: unknown) => handlers.get(event)?.(payload),
    has: (event: string) => handlers.has(event),
  };
}

describe('subscribeChatEvents — subagent delta events', () => {
  let socket: ReturnType<typeof fakeSocket>;

  beforeEach(() => {
    socket = fakeSocket();
    vi.spyOn(socketService, 'getSocket').mockReturnValue(
      socket as unknown as ReturnType<typeof socketService.getSocket>
    );
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('registers and dispatches subagent_text_delta / subagent_thinking_delta', () => {
    const onSubagentTextDelta = vi.fn();
    const onSubagentThinkingDelta = vi.fn();

    const unsubscribe = subscribeChatEvents({ onSubagentTextDelta, onSubagentThinkingDelta });

    expect(socket.has('subagent_text_delta')).toBe(true);
    expect(socket.has('subagent_thinking_delta')).toBe(true);

    const textEvent: ChatSubagentTextDeltaEvent = {
      thread_id: 't1',
      request_id: 'r1',
      round: 1,
      delta: 'hello',
      subagent: { task_id: 'sub-1', agent_id: 'researcher', child_iteration: 1 },
    };
    const thinkingEvent: ChatSubagentThinkingDeltaEvent = {
      thread_id: 't1',
      request_id: 'r1',
      round: 1,
      delta: 'pondering',
      subagent: { task_id: 'sub-1', agent_id: 'researcher', child_iteration: 1 },
    };

    socket.emit('subagent_text_delta', textEvent);
    socket.emit('subagent_thinking_delta', thinkingEvent);

    expect(onSubagentTextDelta).toHaveBeenCalledWith(textEvent);
    expect(onSubagentThinkingDelta).toHaveBeenCalledWith(thinkingEvent);

    unsubscribe();
  });

  it('does not register subagent delta handlers when listeners are omitted', () => {
    subscribeChatEvents({});
    expect(socket.has('subagent_text_delta')).toBe(false);
    expect(socket.has('subagent_thinking_delta')).toBe(false);
  });

  it('returns a no-op unsubscribe when there is no socket', () => {
    vi.spyOn(socketService, 'getSocket').mockReturnValue(
      null as unknown as ReturnType<typeof socketService.getSocket>
    );
    expect(() => subscribeChatEvents({ onSubagentTextDelta: vi.fn() })()).not.toThrow();
  });
});
