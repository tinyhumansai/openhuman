import { renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockListToolkits = vi.fn();
const mockListConnections = vi.fn();

vi.mock('./composioApi', () => ({
  listToolkits: () => mockListToolkits(),
  listConnections: () => mockListConnections(),
}));

describe('useComposioIntegrations', () => {
  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
  });

  it('keeps toolkit cards visible when connections fetch fails', async () => {
    const { useComposioIntegrations } = await import('./hooks');

    mockListToolkits.mockResolvedValue({ toolkits: ['gmail', 'github', 'notion'] });
    mockListConnections.mockRejectedValue(new Error('backend connection listing failed'));

    const { result } = renderHook(() => useComposioIntegrations(0));

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.toolkits).toEqual(['gmail', 'github', 'notion']);
    expect(result.current.connectionByToolkit.size).toBe(0);
    expect(result.current.error).toBe('backend connection listing failed');
  });

  it('surfaces toolkit fetch errors instead of hiding the UI (composio is always enabled)', async () => {
    const { useComposioIntegrations } = await import('./hooks');

    mockListToolkits.mockRejectedValue(new Error('backend unreachable'));
    mockListConnections.mockRejectedValue(new Error('backend unreachable'));

    const { result } = renderHook(() => useComposioIntegrations(0));

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.toolkits).toEqual([]);
    expect(result.current.connectionByToolkit.size).toBe(0);
    expect(result.current.error).toBe('backend unreachable');
  });
});
