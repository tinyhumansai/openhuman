import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockCallCoreRpc = vi.fn();

vi.mock('./coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

// Minimal fixtures -----------------------------------------------------------------

function makeSnapshotResult(overrides: Record<string, unknown> = {}) {
  return {
    auth: { isAuthenticated: true, userId: 'u-1', user: null, profileId: 'p-1' },
    sessionToken: 'tok-abc',
    currentUser: null,
    onboardingCompleted: false,
    analyticsEnabled: true,
    localState: { encryptionKey: null, primaryWalletAddress: null, onboardingTasks: null },
    runtime: { screenIntelligence: {}, localAi: {}, autocomplete: {}, service: {} },
    ...overrides,
  };
}

function makeTeam(id: string) {
  return {
    team: {
      _id: id,
      name: `Team ${id}`,
      slug: id,
      createdBy: 'u-0',
      isPersonal: false,
      maxMembers: 10,
      subscription: { plan: 'FREE' as const, hasActiveSubscription: false },
      usage: { dailyTokenLimit: 0, remainingTokens: 0, activeSessionCount: 0 },
      createdAt: '2026-01-01T00:00:00Z',
      updatedAt: '2026-01-01T00:00:00Z',
    },
    role: 'MEMBER' as const,
  };
}

function makeMember(id: string) {
  return {
    _id: id,
    user: { _id: `u-${id}` },
    role: 'MEMBER' as const,
    joinedAt: '2026-01-01T00:00:00Z',
  };
}

function makeInvite(id: string) {
  return {
    _id: id,
    code: `code-${id}`,
    createdBy: 'u-0',
    expiresAt: '2027-01-01T00:00:00Z',
    maxUses: 10,
    currentUses: 0,
    usageHistory: [],
  };
}

// Tests ----------------------------------------------------------------------------

describe('coreStateApi.fetchCoreAppSnapshot', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('calls the correct RPC method', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: makeSnapshotResult() });

    const { fetchCoreAppSnapshot } = await import('./coreStateApi');
    await fetchCoreAppSnapshot();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.app_state_snapshot' });
  });

  it('returns the inner result from the RPC envelope', async () => {
    const snapshot = makeSnapshotResult({ sessionToken: 'tok-xyz' });
    mockCallCoreRpc.mockResolvedValueOnce({ result: snapshot });

    const { fetchCoreAppSnapshot } = await import('./coreStateApi');
    const out = await fetchCoreAppSnapshot();

    expect(out.sessionToken).toBe('tok-xyz');
    expect(out.auth.isAuthenticated).toBe(true);
    expect(out.auth.userId).toBe('u-1');
  });

  it('propagates rejection from callCoreRpc', async () => {
    mockCallCoreRpc.mockRejectedValueOnce(new Error('snapshot failed'));

    const { fetchCoreAppSnapshot } = await import('./coreStateApi');
    await expect(fetchCoreAppSnapshot()).rejects.toThrow('snapshot failed');
  });
});

describe('coreStateApi.updateCoreLocalState', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('calls the correct RPC method with params', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({});

    const { updateCoreLocalState } = await import('./coreStateApi');
    const params = { encryptionKey: 'key-123', primaryWalletAddress: '0xABCD' };
    await updateCoreLocalState(params);

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.app_state_update_local_state',
      params,
    });
  });

  it('resolves without a return value on success', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({});

    const { updateCoreLocalState } = await import('./coreStateApi');
    const result = await updateCoreLocalState({ encryptionKey: null });
    expect(result).toBeUndefined();
  });

  it('passes null fields correctly to the RPC', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({});

    const { updateCoreLocalState } = await import('./coreStateApi');
    await updateCoreLocalState({
      encryptionKey: null,
      primaryWalletAddress: null,
      onboardingTasks: null,
    });

    const call = mockCallCoreRpc.mock.calls[0][0] as { params: unknown };
    expect(call.params).toEqual({
      encryptionKey: null,
      primaryWalletAddress: null,
      onboardingTasks: null,
    });
  });

  it('propagates rejection from callCoreRpc', async () => {
    mockCallCoreRpc.mockRejectedValueOnce(new Error('update rejected'));

    const { updateCoreLocalState } = await import('./coreStateApi');
    await expect(updateCoreLocalState({})).rejects.toThrow('update rejected');
  });
});

describe('coreStateApi.listTeams', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('calls team_list_teams RPC and returns teams array', async () => {
    const teams = [makeTeam('t-1'), makeTeam('t-2')];
    mockCallCoreRpc.mockResolvedValueOnce({ result: teams });

    const { listTeams } = await import('./coreStateApi');
    const out = await listTeams();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.team_list_teams' });
    expect(out).toHaveLength(2);
    expect(out[0].team._id).toBe('t-1');
  });

  it('returns empty array when no teams exist', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: [] });

    const { listTeams } = await import('./coreStateApi');
    const out = await listTeams();

    expect(out).toEqual([]);
  });
});

describe('coreStateApi.getTeamMembers', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('calls team_list_members with teamId param', async () => {
    const members = [makeMember('m-1')];
    mockCallCoreRpc.mockResolvedValueOnce({ result: members });

    const { getTeamMembers } = await import('./coreStateApi');
    const out = await getTeamMembers('t-1');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.team_list_members',
      params: { teamId: 't-1' },
    });
    expect(out[0]._id).toBe('m-1');
  });

  it('returns the inner result array', async () => {
    const members = [makeMember('a'), makeMember('b')];
    mockCallCoreRpc.mockResolvedValueOnce({ result: members });

    const { getTeamMembers } = await import('./coreStateApi');
    const out = await getTeamMembers('t-99');

    expect(out).toHaveLength(2);
  });
});

describe('coreStateApi.getTeamInvites', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('calls team_list_invites with teamId param', async () => {
    const invites = [makeInvite('inv-1')];
    mockCallCoreRpc.mockResolvedValueOnce({ result: invites });

    const { getTeamInvites } = await import('./coreStateApi');
    const out = await getTeamInvites('t-1');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.team_list_invites',
      params: { teamId: 't-1' },
    });
    expect(out[0]._id).toBe('inv-1');
  });

  it('returns empty array when no invites exist', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: [] });

    const { getTeamInvites } = await import('./coreStateApi');
    const out = await getTeamInvites('t-empty');

    expect(out).toEqual([]);
  });
});
