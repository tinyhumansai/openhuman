import { afterEach, describe, expect, it, vi } from 'vitest';

import { socketService } from '../socketService';

/**
 * `subscribeThread` is what orders the reconnect recovery in #6034: the chat
 * runtime re-reads a thread only after the room join is acknowledged, and it
 * keeps a thread queued for the next connection when the join never happened.
 * Both of those decisions read this function's resolved value, so the three
 * outcomes are pinned here rather than through the provider (which mocks this
 * module out entirely).
 */

type FakeSocket = { connected: boolean; emit: ReturnType<typeof vi.fn> };

/** Install a stand-in for the private socket the singleton holds. */
function withSocket(socket: FakeSocket | null) {
  (socketService as unknown as { socket: FakeSocket | null }).socket = socket;
}

afterEach(() => {
  withSocket(null);
  vi.useRealTimers();
});

describe('socketService.subscribeThread (#6034)', () => {
  it('resolves true once the server acknowledges the room join', async () => {
    const emit = vi.fn((_event: string, _payload: unknown, ack: () => void) => ack());
    withSocket({ connected: true, emit });

    await expect(socketService.subscribeThread('t-1')).resolves.toBe(true);
    expect(emit).toHaveBeenCalledWith(
      'thread:subscribe',
      { thread_id: 't-1' },
      expect.any(Function)
    );
  });

  it('resolves false without emitting when the socket is not connected', async () => {
    const emit = vi.fn();
    withSocket({ connected: false, emit });

    // The caller keeps the thread queued for the next connection on a false —
    // clearing it here is what stranded a thread whose socket dropped again.
    await expect(socketService.subscribeThread('t-2')).resolves.toBe(false);
    expect(emit).not.toHaveBeenCalled();
  });

  it('resolves false when the acknowledgement never arrives', async () => {
    vi.useFakeTimers();
    // A core without the ack handler never calls back. Recovery must still
    // proceed rather than hang, so the wait is bounded.
    const emit = vi.fn();
    withSocket({ connected: true, emit });

    const pending = socketService.subscribeThread('t-3', 50);
    await vi.advanceTimersByTimeAsync(60);
    await expect(pending).resolves.toBe(false);
    expect(emit).toHaveBeenCalledTimes(1);
  });

  it('resolves false for an empty thread id and sends nothing', async () => {
    const emit = vi.fn();
    withSocket({ connected: true, emit });

    await expect(socketService.subscribeThread('')).resolves.toBe(false);
    expect(emit).not.toHaveBeenCalled();
  });

  it('does not resolve twice when the ack lands after the timeout', async () => {
    vi.useFakeTimers();
    let late: (() => void) | undefined;
    const emit = vi.fn((_event: string, _payload: unknown, ack: () => void) => {
      late = ack;
    });
    withSocket({ connected: true, emit });

    const pending = socketService.subscribeThread('t-4', 50);
    await vi.advanceTimersByTimeAsync(60);
    late?.();

    await expect(pending).resolves.toBe(false);
  });
});
