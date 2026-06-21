import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { useTinyplaceStream } from './useTinyplaceStream';

// ── Mock socketService ────────────────────────────────────────────────────────

const onListeners = new Map<string, Set<(...args: unknown[]) => void>>();

vi.mock('../../services/socketService', () => ({
  socketService: {
    on: vi.fn((event: string, cb: (...args: unknown[]) => void) => {
      if (!onListeners.has(event)) onListeners.set(event, new Set());
      onListeners.get(event)!.add(cb);
    }),
    off: vi.fn((event: string, cb: (...args: unknown[]) => void) => {
      onListeners.get(event)?.delete(cb);
    }),
  },
}));

function emit(event: string, data: unknown) {
  for (const cb of onListeners.get(event) ?? []) {
    cb(data);
  }
}

beforeEach(() => {
  vi.clearAllMocks();
  onListeners.clear();
});

// ── Tests ─────────────────────────────────────────────────────────────────────

describe('useTinyplaceStream', () => {
  test('starts with idle status and empty messages', () => {
    const { result } = renderHook(() => useTinyplaceStream('inbox'));
    expect(result.current.status).toBe('idle');
    expect(result.current.messages).toEqual([]);
  });

  test('updates status on tinyplace:stream_status event', () => {
    const { result } = renderHook(() => useTinyplaceStream('inbox'));
    act(() => {
      emit('tinyplace:stream_status', { stream_id: 'inbox', status: 'connected' });
    });
    expect(result.current.status).toBe('connected');
  });

  test('ignores status events for other stream ids', () => {
    const { result } = renderHook(() => useTinyplaceStream('inbox'));
    act(() => {
      emit('tinyplace:stream_status', { stream_id: 'conversation:abc', status: 'connected' });
    });
    expect(result.current.status).toBe('idle');
  });

  test('collects stream messages', () => {
    const { result } = renderHook(() => useTinyplaceStream('inbox'));
    act(() => {
      emit('tinyplace:stream_message', {
        stream_id: 'inbox',
        kind: 'inbox',
        message: { itemId: '1', type: 'conversation_message' },
      });
    });
    expect(result.current.messages).toHaveLength(1);
    expect((result.current.messages[0].message as Record<string, unknown>).itemId).toBe('1');
  });

  test('ignores messages for other stream ids', () => {
    const { result } = renderHook(() => useTinyplaceStream('inbox'));
    act(() => {
      emit('tinyplace:stream_message', {
        stream_id: 'conversation:xyz',
        kind: 'conversation',
        message: { id: '99' },
      });
    });
    expect(result.current.messages).toHaveLength(0);
  });

  test('caps messages at 100', () => {
    const { result } = renderHook(() => useTinyplaceStream('inbox'));
    act(() => {
      for (let i = 0; i < 110; i++) {
        emit('tinyplace:stream_message', {
          stream_id: 'inbox',
          kind: 'inbox',
          message: { itemId: String(i) },
        });
      }
    });
    expect(result.current.messages.length).toBeLessThanOrEqual(100);
  });

  test('clearMessages resets the list', () => {
    const { result } = renderHook(() => useTinyplaceStream('inbox'));
    act(() => {
      emit('tinyplace:stream_message', {
        stream_id: 'inbox',
        kind: 'inbox',
        message: { itemId: '1' },
      });
    });
    expect(result.current.messages).toHaveLength(1);
    act(() => {
      result.current.clearMessages();
    });
    expect(result.current.messages).toHaveLength(0);
  });

  test('unsubscribes on unmount', () => {
    const { unmount } = renderHook(() => useTinyplaceStream('inbox'));
    // Listeners should be registered.
    expect(onListeners.get('tinyplace:stream_message')?.size).toBeGreaterThan(0);
    unmount();
    // Listeners should be cleaned up.
    expect(onListeners.get('tinyplace:stream_message')?.size ?? 0).toBe(0);
  });

  test('accepts messages for all stream ids when no filter is given', () => {
    const { result } = renderHook(() => useTinyplaceStream());
    act(() => {
      emit('tinyplace:stream_message', {
        stream_id: 'inbox',
        kind: 'inbox',
        message: { itemId: 'A' },
      });
      emit('tinyplace:stream_message', {
        stream_id: 'conversation:123',
        kind: 'conversation',
        message: { id: 'B' },
      });
    });
    expect(result.current.messages).toHaveLength(2);
  });
});
