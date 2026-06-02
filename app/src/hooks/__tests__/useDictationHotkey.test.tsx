// @vitest-environment jsdom
import { renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { useDictationHotkey } from '../useDictationHotkey';

const hoisted = vi.hoisted(() => {
  const handlers: Record<string, (...args: unknown[]) => void> = {};
  const mockSocket = {
    on: vi.fn((event: string, cb: (...args: unknown[]) => void) => {
      handlers[event] = cb;
    }),
    off: vi.fn(),
    disconnect: vi.fn(),
    id: 'mock-sid',
  };
  return {
    handlers,
    mockSocket,
    connectCoreSocketMock: vi
      .fn<() => Promise<typeof mockSocket | null>>()
      .mockResolvedValue(mockSocket),
    callCoreRpcMock: vi.fn<() => Promise<unknown>>(),
    getCoreHttpBaseUrlMock: vi.fn(async () => 'http://127.0.0.1:7788'),
  };
});

vi.mock('../../services/coreSocket', () => ({ connectCoreSocket: hoisted.connectCoreSocketMock }));
vi.mock('../../services/coreRpcClient', () => ({
  callCoreRpc: hoisted.callCoreRpcMock,
  getCoreHttpBaseUrl: hoisted.getCoreHttpBaseUrlMock,
}));

describe('useDictationHotkey', () => {
  beforeEach(() => {
    hoisted.connectCoreSocketMock.mockClear();
    hoisted.connectCoreSocketMock.mockResolvedValue(hoisted.mockSocket);
    hoisted.callCoreRpcMock.mockClear();
    hoisted.callCoreRpcMock.mockResolvedValue({
      enabled: true,
      hotkey: 'F1',
      activationMode: 'toggle',
    });
    hoisted.mockSocket.on.mockClear();
    hoisted.mockSocket.off.mockClear();
    hoisted.mockSocket.disconnect.mockClear();
    Object.keys(hoisted.handlers).forEach(k => delete hoisted.handlers[k]);
  });

  it('opens a dedicated core socket on mount via connectCoreSocket', async () => {
    renderHook(() => useDictationHotkey());

    await waitFor(() => {
      expect(hoisted.connectCoreSocketMock).toHaveBeenCalledTimes(1);
    });

    const args = hoisted.connectCoreSocketMock.mock.calls[0] as unknown as [
      { getBaseUrl: () => Promise<string>; isDisposed: () => boolean },
    ];
    expect(typeof args[0].getBaseUrl).toBe('function');
    expect(typeof args[0].isDisposed).toBe('function');
    expect(args[0].isDisposed()).toBe(false);
  });

  it('disconnects the socket on unmount', async () => {
    const { unmount } = renderHook(() => useDictationHotkey());
    await waitFor(() => {
      expect(hoisted.connectCoreSocketMock).toHaveBeenCalled();
    });
    unmount();
    expect(hoisted.mockSocket.disconnect).toHaveBeenCalled();
  });

  it('short-circuits when connectCoreSocket returns null (disposed mid-await)', async () => {
    hoisted.connectCoreSocketMock.mockResolvedValueOnce(null);
    renderHook(() => useDictationHotkey());
    await waitFor(() => {
      expect(hoisted.connectCoreSocketMock).toHaveBeenCalled();
    });
    expect(hoisted.mockSocket.on).not.toHaveBeenCalled();
  });

  it('dispatches an autoSend insert-text event on transcription', async () => {
    renderHook(() => useDictationHotkey());
    await waitFor(() => {
      expect(hoisted.handlers['dictation:transcription']).toBeDefined();
    });

    const received: CustomEvent<{ text?: string; autoSend?: boolean }>[] = [];
    const listener = (e: Event) =>
      received.push(e as CustomEvent<{ text?: string; autoSend?: boolean }>);
    window.addEventListener('dictation://insert-text', listener);
    try {
      hoisted.handlers['dictation:transcription']({ text: '  hello world  ' });
    } finally {
      window.removeEventListener('dictation://insert-text', listener);
    }

    expect(received).toHaveLength(1);
    // Trimmed text, and autoSend flag set so Conversations sends it straight to the agent.
    expect(received[0].detail).toEqual({ text: 'hello world', autoSend: true });
  });

  it('ignores blank transcription text (no event dispatched)', async () => {
    renderHook(() => useDictationHotkey());
    await waitFor(() => {
      expect(hoisted.handlers['dictation:transcription']).toBeDefined();
    });

    const received: Event[] = [];
    const listener = (e: Event) => received.push(e);
    window.addEventListener('dictation://insert-text', listener);
    try {
      hoisted.handlers['dictation:transcription']({ text: '   ' });
    } finally {
      window.removeEventListener('dictation://insert-text', listener);
    }

    expect(received).toHaveLength(0);
  });
});
