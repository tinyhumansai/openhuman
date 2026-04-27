import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockCallCoreCommand = vi.fn();

vi.mock('../coreCommandClient', () => ({
  callCoreCommand: (...args: unknown[]) => mockCallCoreCommand(...args),
}));

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

function makeTunnel(id: string) {
  return {
    id,
    uuid: `uuid-${id}`,
    name: `Tunnel ${id}`,
    description: 'A test tunnel',
    isActive: true,
    createdAt: '2026-01-01T00:00:00Z',
    updatedAt: '2026-01-01T00:00:00Z',
  };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('tunnelsApi.createTunnel', () => {
  beforeEach(() => {
    mockCallCoreCommand.mockReset();
  });

  it('calls webhooks_create_tunnel with the request body', async () => {
    const tunnel = makeTunnel('t-1');
    mockCallCoreCommand.mockResolvedValueOnce(tunnel);

    const { tunnelsApi } = await import('./tunnelsApi');
    const result = await tunnelsApi.createTunnel({ name: 'My Tunnel', description: 'desc' });

    expect(mockCallCoreCommand).toHaveBeenCalledWith('openhuman.webhooks_create_tunnel', {
      name: 'My Tunnel',
      description: 'desc',
    });
    expect(result.id).toBe('t-1');
    expect(result.name).toBe('Tunnel t-1');
  });

  it('creates a tunnel without an optional description', async () => {
    mockCallCoreCommand.mockResolvedValueOnce(makeTunnel('t-2'));

    const { tunnelsApi } = await import('./tunnelsApi');
    await tunnelsApi.createTunnel({ name: 'No Desc' });

    expect(mockCallCoreCommand).toHaveBeenCalledWith('openhuman.webhooks_create_tunnel', {
      name: 'No Desc',
    });
  });

  it('propagates rejection from callCoreCommand', async () => {
    mockCallCoreCommand.mockRejectedValueOnce(new Error('create failed'));

    const { tunnelsApi } = await import('./tunnelsApi');
    await expect(tunnelsApi.createTunnel({ name: 'bad' })).rejects.toThrow('create failed');
  });
});

describe('tunnelsApi.getTunnels', () => {
  beforeEach(() => {
    mockCallCoreCommand.mockReset();
  });

  it('calls webhooks_list_tunnels with no params', async () => {
    const tunnels = [makeTunnel('t-1'), makeTunnel('t-2')];
    mockCallCoreCommand.mockResolvedValueOnce(tunnels);

    const { tunnelsApi } = await import('./tunnelsApi');
    const result = await tunnelsApi.getTunnels();

    expect(mockCallCoreCommand).toHaveBeenCalledWith('openhuman.webhooks_list_tunnels');
    expect(result).toHaveLength(2);
  });

  it('returns empty array when no tunnels exist', async () => {
    mockCallCoreCommand.mockResolvedValueOnce([]);

    const { tunnelsApi } = await import('./tunnelsApi');
    const result = await tunnelsApi.getTunnels();

    expect(result).toEqual([]);
  });
});

describe('tunnelsApi.getBandwidthUsage', () => {
  beforeEach(() => {
    mockCallCoreCommand.mockReset();
  });

  it('calls webhooks_get_bandwidth and returns budget info', async () => {
    mockCallCoreCommand.mockResolvedValueOnce({ remainingBudgetUsd: 4.5 });

    const { tunnelsApi } = await import('./tunnelsApi');
    const result = await tunnelsApi.getBandwidthUsage();

    expect(mockCallCoreCommand).toHaveBeenCalledWith('openhuman.webhooks_get_bandwidth');
    expect(result.remainingBudgetUsd).toBe(4.5);
  });
});

describe('tunnelsApi.getTunnel', () => {
  beforeEach(() => {
    mockCallCoreCommand.mockReset();
  });

  it('calls webhooks_get_tunnel with the tunnel ID', async () => {
    mockCallCoreCommand.mockResolvedValueOnce(makeTunnel('t-99'));

    const { tunnelsApi } = await import('./tunnelsApi');
    const result = await tunnelsApi.getTunnel('t-99');

    expect(mockCallCoreCommand).toHaveBeenCalledWith('openhuman.webhooks_get_tunnel', {
      id: 't-99',
    });
    expect(result.id).toBe('t-99');
  });

  it('propagates rejection when tunnel is not found', async () => {
    mockCallCoreCommand.mockRejectedValueOnce(new Error('not found'));

    const { tunnelsApi } = await import('./tunnelsApi');
    await expect(tunnelsApi.getTunnel('missing')).rejects.toThrow('not found');
  });
});

describe('tunnelsApi.updateTunnel', () => {
  beforeEach(() => {
    mockCallCoreCommand.mockReset();
  });

  it('calls webhooks_update_tunnel merging id with body fields', async () => {
    const updated = { ...makeTunnel('t-1'), name: 'Renamed' };
    mockCallCoreCommand.mockResolvedValueOnce(updated);

    const { tunnelsApi } = await import('./tunnelsApi');
    const result = await tunnelsApi.updateTunnel('t-1', { name: 'Renamed' });

    expect(mockCallCoreCommand).toHaveBeenCalledWith('openhuman.webhooks_update_tunnel', {
      id: 't-1',
      name: 'Renamed',
    });
    expect(result.name).toBe('Renamed');
  });

  it('can set isActive to false', async () => {
    const deactivated = { ...makeTunnel('t-2'), isActive: false };
    mockCallCoreCommand.mockResolvedValueOnce(deactivated);

    const { tunnelsApi } = await import('./tunnelsApi');
    const result = await tunnelsApi.updateTunnel('t-2', { isActive: false });

    expect(mockCallCoreCommand).toHaveBeenCalledWith('openhuman.webhooks_update_tunnel', {
      id: 't-2',
      isActive: false,
    });
    expect(result.isActive).toBe(false);
  });

  it('propagates rejection from callCoreCommand', async () => {
    mockCallCoreCommand.mockRejectedValueOnce(new Error('update failed'));

    const { tunnelsApi } = await import('./tunnelsApi');
    await expect(tunnelsApi.updateTunnel('t-bad', { name: 'x' })).rejects.toThrow('update failed');
  });
});

describe('tunnelsApi.deleteTunnel', () => {
  beforeEach(() => {
    mockCallCoreCommand.mockReset();
  });

  it('calls webhooks_delete_tunnel with the tunnel ID', async () => {
    mockCallCoreCommand.mockResolvedValueOnce(undefined);

    const { tunnelsApi } = await import('./tunnelsApi');
    await tunnelsApi.deleteTunnel('t-1');

    expect(mockCallCoreCommand).toHaveBeenCalledWith('openhuman.webhooks_delete_tunnel', {
      id: 't-1',
    });
  });

  it('resolves without a return value on success', async () => {
    mockCallCoreCommand.mockResolvedValueOnce(undefined);

    const { tunnelsApi } = await import('./tunnelsApi');
    const result = await tunnelsApi.deleteTunnel('t-1');
    expect(result).toBeUndefined();
  });

  it('propagates rejection from callCoreCommand', async () => {
    mockCallCoreCommand.mockRejectedValueOnce(new Error('delete failed'));

    const { tunnelsApi } = await import('./tunnelsApi');
    await expect(tunnelsApi.deleteTunnel('t-bad')).rejects.toThrow('delete failed');
  });
});

describe('tunnelsApi.ingressUrl', () => {
  it('builds the correct ingress URL from backend URL and tunnel UUID', async () => {
    const { tunnelsApi } = await import('./tunnelsApi');
    const url = tunnelsApi.ingressUrl('https://api.example.com', 'uuid-abc');
    expect(url).toBe('https://api.example.com/webhooks/ingress/uuid-abc');
  });

  it('strips trailing slash from backendUrl before building the URL', async () => {
    const { tunnelsApi } = await import('./tunnelsApi');
    const url = tunnelsApi.ingressUrl('https://api.example.com/', 'uuid-xyz');
    expect(url).toBe('https://api.example.com/webhooks/ingress/uuid-xyz');
  });

  it('works with localhost URLs', async () => {
    const { tunnelsApi } = await import('./tunnelsApi');
    const url = tunnelsApi.ingressUrl('http://localhost:5005', 'test-uuid');
    expect(url).toBe('http://localhost:5005/webhooks/ingress/test-uuid');
  });
});
