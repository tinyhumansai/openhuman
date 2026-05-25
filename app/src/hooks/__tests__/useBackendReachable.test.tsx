import { renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { useBackendReachable } from '../useBackendReachable';

vi.mock('../../services/backendUrl', () => ({
  getBackendUrl: vi.fn().mockResolvedValue('http://localhost:5005'),
}));

describe('useBackendReachable', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it('reports reachable when fetch resolves with any HTTP response', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response(null, { status: 200 }));

    const { result } = renderHook(() => useBackendReachable());
    expect(result.current).toBe('probing');
    await waitFor(() => expect(result.current).toBe('reachable'));
  });

  it('reports reachable on 4xx (host answered)', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response(null, { status: 404 }));

    const { result } = renderHook(() => useBackendReachable());
    await waitFor(() => expect(result.current).toBe('reachable'));
  });

  it('reports unreachable when fetch rejects (network error)', async () => {
    vi.spyOn(globalThis, 'fetch').mockRejectedValue(new TypeError('Failed to fetch'));

    const { result } = renderHook(() => useBackendReachable());
    await waitFor(() => expect(result.current).toBe('unreachable'));
  });
});
