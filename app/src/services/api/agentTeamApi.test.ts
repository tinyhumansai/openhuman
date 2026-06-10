import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../coreRpcClient';
import { agentTeamApi } from './agentTeamApi';

vi.mock('../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const mockCall = vi.mocked(callCoreRpc);

beforeEach(() => {
  vi.clearAllMocks();
});

describe('agentTeamApi.list', () => {
  it('unwraps the { teams, count } envelope to an array', async () => {
    mockCall.mockResolvedValueOnce({ teams: [{ id: 't1' }, { id: 't2' }], count: 2 });
    const teams = await agentTeamApi.list();
    expect(teams).toHaveLength(2);
    expect(teams[0].id).toBe('t1');
    expect(mockCall).toHaveBeenCalledWith({ method: 'openhuman.agent_team_list', params: {} });
  });

  it('returns [] when the envelope omits teams', async () => {
    mockCall.mockResolvedValueOnce({ count: 0 });
    expect(await agentTeamApi.list()).toEqual([]);
  });

  it('forwards filters as params', async () => {
    mockCall.mockResolvedValueOnce({ teams: [], count: 0 });
    await agentTeamApi.list({ parentThreadId: 'thread-1', status: 'active', limit: 10 });
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.agent_team_list',
      params: { parentThreadId: 'thread-1', status: 'active', limit: 10 },
    });
  });

  it('rejects a non-positive limit without calling core', async () => {
    await expect(agentTeamApi.list({ limit: 0 })).rejects.toThrow('positive integer');
    await expect(agentTeamApi.list({ limit: 1.5 })).rejects.toThrow('positive integer');
    expect(mockCall).not.toHaveBeenCalled();
  });
});

describe('agentTeamApi.get', () => {
  it('unwraps { team } to the TeamView', async () => {
    const view = { team: { id: 't1' }, members: [], tasks: [] };
    mockCall.mockResolvedValueOnce({ team: view });
    const result = await agentTeamApi.get('t1');
    expect(result).toBe(view);
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.agent_team_get',
      params: { teamId: 't1' },
    });
  });

  it('returns null when the team is unknown ({ team: null })', async () => {
    mockCall.mockResolvedValueOnce({ team: null });
    expect(await agentTeamApi.get('missing')).toBeNull();
  });

  it('throws when teamId is empty, without calling core', async () => {
    await expect(agentTeamApi.get('')).rejects.toThrow('teamId is required');
    expect(mockCall).not.toHaveBeenCalled();
  });
});

describe('agentTeamApi.listMessages', () => {
  it('unwraps { messages } and narrows each run-event payload', async () => {
    mockCall.mockResolvedValueOnce({
      messages: [
        {
          runId: 'team-1',
          sequence: 1,
          eventType: 'team_message',
          payload: { from: 'm1', to: 'm2', content: 'hi', visibility: 'team' },
          timestamp: '2026-01-01T00:00:00Z',
        },
      ],
    });
    const messages = await agentTeamApi.listMessages('team-1');
    expect(messages).toHaveLength(1);
    expect(messages[0].payload).toEqual({
      from: 'm1',
      to: 'm2',
      content: 'hi',
      visibility: 'team',
    });
  });

  it('defends against a malformed payload (missing fields → safe defaults)', async () => {
    mockCall.mockResolvedValueOnce({
      messages: [
        {
          runId: 'team-1',
          sequence: 2,
          eventType: 'team_message',
          payload: { from: 'm1' },
          timestamp: '2026-01-01T00:00:00Z',
        },
      ],
    });
    const [msg] = await agentTeamApi.listMessages('team-1');
    expect(msg.payload).toEqual({ from: 'm1', to: null, content: '', visibility: 'team' });
  });

  it('returns [] when the envelope omits messages', async () => {
    mockCall.mockResolvedValueOnce({});
    expect(await agentTeamApi.listMessages('team-1')).toEqual([]);
  });

  it('forwards an explicit limit, omits it otherwise', async () => {
    mockCall.mockResolvedValueOnce({ messages: [] });
    await agentTeamApi.listMessages('team-1', 50);
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.agent_team_list_messages',
      params: { teamId: 'team-1', limit: 50 },
    });
    mockCall.mockResolvedValueOnce({ messages: [] });
    await agentTeamApi.listMessages('team-1');
    expect(mockCall).toHaveBeenLastCalledWith({
      method: 'openhuman.agent_team_list_messages',
      params: { teamId: 'team-1' },
    });
  });
});
