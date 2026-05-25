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
});
