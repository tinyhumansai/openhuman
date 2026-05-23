import { renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockListToolkits = vi.fn();
const mockListConnections = vi.fn();
const mockListAgentReadyToolkits = vi.fn();

vi.mock('./composioApi', () => ({
  listToolkits: () => mockListToolkits(),
  listConnections: () => mockListConnections(),
  listAgentReadyToolkits: () => mockListAgentReadyToolkits(),
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

describe('useAgentReadyComposioToolkits', () => {
  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
  });

  it('returns a normalized Set of agent-ready toolkit slugs on success', async () => {
    const { useAgentReadyComposioToolkits } = await import('./hooks');

    mockListAgentReadyToolkits.mockResolvedValue({
      toolkits: ['gmail', 'one_drive', 'EXCEL', 'todoist'],
    });

    const { result } = renderHook(() => useAgentReadyComposioToolkits());

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    // canonicalizeComposioToolkitSlug normalizes case and aliases.
    expect(result.current.agentReady.has('gmail')).toBe(true);
    expect(result.current.agentReady.has('one_drive')).toBe(true);
    expect(result.current.agentReady.has('excel')).toBe(true);
    expect(result.current.agentReady.has('todoist')).toBe(true);
    // Uncatalogued toolkit must NOT appear — the UI relies on this
    // to drive the preview-badge logic (issue #2283).
    expect(result.current.agentReady.has('clickup')).toBe(false);
    expect(result.current.error).toBeNull();
  });

  it('returns an empty set and surfaces error when the RPC fails', async () => {
    const { useAgentReadyComposioToolkits } = await import('./hooks');

    mockListAgentReadyToolkits.mockRejectedValue(new Error('rpc unavailable'));

    const { result } = renderHook(() => useAgentReadyComposioToolkits());

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    // Failure must NOT label every toolkit as preview — surface the
    // error and let the caller decide how to degrade.
    expect(result.current.agentReady.size).toBe(0);
    expect(result.current.error).toBe('rpc unavailable');
  });

  it('handles a missing toolkits field without throwing', async () => {
    const { useAgentReadyComposioToolkits } = await import('./hooks');

    mockListAgentReadyToolkits.mockResolvedValue({});

    const { result } = renderHook(() => useAgentReadyComposioToolkits());

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.agentReady.size).toBe(0);
    expect(result.current.error).toBeNull();
  });
});
