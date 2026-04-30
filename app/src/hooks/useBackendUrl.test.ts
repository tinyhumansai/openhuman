import { renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockGetBackendUrl = vi.fn();

vi.mock('../services/backendUrl', () => ({ getBackendUrl: () => mockGetBackendUrl() }));

describe('useBackendUrl', () => {
  beforeEach(() => {
    vi.resetModules();
    mockGetBackendUrl.mockReset();
  });

  it('returns the resolved core-derived backend URL after the async lookup settles', async () => {
    mockGetBackendUrl.mockResolvedValue('https://api.example.com');

    const { useBackendUrl } = await import('./useBackendUrl');
    const { result } = renderHook(() => useBackendUrl());

    expect(result.current).toBeNull();
    await waitFor(() => expect(result.current).toBe('https://api.example.com'));
  });

  it('stays null when resolution fails so callers do not fall back to a hardcoded host', async () => {
    mockGetBackendUrl.mockRejectedValue(new Error('rpc unreachable'));

    const { useBackendUrl } = await import('./useBackendUrl');
    const { result } = renderHook(() => useBackendUrl());

    await waitFor(() => expect(mockGetBackendUrl).toHaveBeenCalled());
    expect(result.current).toBeNull();
  });

  it('does not update state if the component unmounts before resolution', async () => {
    let resolveFn: ((value: string) => void) | undefined;
    mockGetBackendUrl.mockImplementation(
      () =>
        new Promise<string>(resolve => {
          resolveFn = resolve;
        })
    );

    const { useBackendUrl } = await import('./useBackendUrl');
    const { result, unmount } = renderHook(() => useBackendUrl());

    unmount();
    resolveFn?.('https://api.example.com');

    await new Promise(r => setTimeout(r, 10));
    expect(result.current).toBeNull();
  });
});
