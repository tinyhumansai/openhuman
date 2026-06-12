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

describe('agentTeamApi.completeTask', () => {
  it('unwraps { result } and defaults evidence + requireEvidence', async () => {
    mockCall.mockResolvedValueOnce({ result: { kind: 'completed', id: 'task-1', status: 'done' } });
    const outcome = await agentTeamApi.completeTask({
      teamId: 'team-1',
      taskId: 'task-1',
      memberId: 'm1',
    });
    expect(outcome.kind).toBe('completed');
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.agent_team_complete_task',
      params: {
        teamId: 'team-1',
        taskId: 'task-1',
        memberId: 'm1',
        evidence: [],
        requireEvidence: false,
      },
    });
  });

  it('forwards evidence + requireEvidence and surfaces a gateFailed outcome', async () => {
    mockCall.mockResolvedValueOnce({
      result: { kind: 'gateFailed', reasons: ['completion requires at least one evidence link'] },
    });
    const outcome = await agentTeamApi.completeTask({
      teamId: 'team-1',
      taskId: 'task-1',
      memberId: 'm1',
      evidence: ['proof'],
      requireEvidence: true,
    });
    expect(outcome).toEqual({
      kind: 'gateFailed',
      reasons: ['completion requires at least one evidence link'],
    });
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.agent_team_complete_task',
      params: {
        teamId: 'team-1',
        taskId: 'task-1',
        memberId: 'm1',
        evidence: ['proof'],
        requireEvidence: true,
      },
    });
  });

  it('throws when a required id is missing, without calling core', async () => {
    await expect(
      agentTeamApi.completeTask({ teamId: '', taskId: 't', memberId: 'm' })
    ).rejects.toThrow('required');
    expect(mockCall).not.toHaveBeenCalled();
  });
});

describe('agentTeamApi.shutdownMember', () => {
  it('unwraps { result } to the MemberShutdown', async () => {
    const result = { member: { id: 'm1', memberStatus: 'stopped' }, releasedTaskIds: ['task-1'] };
    mockCall.mockResolvedValueOnce({ result });
    const outcome = await agentTeamApi.shutdownMember('team-1', 'm1');
    expect(outcome).toBe(result);
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.agent_team_shutdown_member',
      params: { teamId: 'team-1', memberId: 'm1' },
    });
  });

  it('throws when teamId or memberId is empty, without calling core', async () => {
    await expect(agentTeamApi.shutdownMember('', 'm1')).rejects.toThrow('required');
    await expect(agentTeamApi.shutdownMember('team-1', '')).rejects.toThrow('required');
    expect(mockCall).not.toHaveBeenCalled();
  });
});

describe('agentTeamApi.messageMember', () => {
  it('narrows the { message } event and omits absent optional fields', async () => {
    mockCall.mockResolvedValueOnce({
      message: {
        runId: 'team-1',
        sequence: 5,
        eventType: 'team_message',
        payload: { from: 'lead', to: 'm1', content: 'go', visibility: 'team' },
        timestamp: '2026-01-01T00:00:00Z',
      },
    });
    const msg = await agentTeamApi.messageMember({
      teamId: 'team-1',
      toMemberId: 'm1',
      content: 'go',
    });
    expect(msg.payload).toEqual({ from: 'lead', to: 'm1', content: 'go', visibility: 'team' });
    // fromMemberId omitted → lead origin; no empty keys forwarded.
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.agent_team_message_member',
      params: { teamId: 'team-1', content: 'go', toMemberId: 'm1' },
    });
  });

  it('forwards fromMemberId for teammate-to-teammate messages', async () => {
    mockCall.mockResolvedValueOnce({
      message: {
        runId: 'team-1',
        sequence: 6,
        eventType: 'team_message',
        payload: { from: 'm1', to: 'm2', content: 'hi', visibility: 'team' },
        timestamp: '2026-01-01T00:00:00Z',
      },
    });
    await agentTeamApi.messageMember({
      teamId: 'team-1',
      fromMemberId: 'm1',
      toMemberId: 'm2',
      content: 'hi',
    });
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.agent_team_message_member',
      params: { teamId: 'team-1', content: 'hi', fromMemberId: 'm1', toMemberId: 'm2' },
    });
  });

  it('throws on empty content or teamId, without calling core', async () => {
    await expect(agentTeamApi.messageMember({ teamId: '', content: 'x' })).rejects.toThrow(
      'teamId is required'
    );
    await expect(agentTeamApi.messageMember({ teamId: 'team-1', content: '  ' })).rejects.toThrow(
      'content is required'
    );
    expect(mockCall).not.toHaveBeenCalled();
  });
});

describe('agentTeamApi.startMember', () => {
  it('unwraps { result } and forwards an explicit taskId', async () => {
    mockCall.mockResolvedValueOnce({
      result: { kind: 'started', runId: 'teamrun-1', task: { id: 'task-1' } },
    });
    const outcome = await agentTeamApi.startMember({
      teamId: 'team-1',
      memberId: 'm1',
      taskId: 'task-1',
    });
    expect(outcome).toEqual({ kind: 'started', runId: 'teamrun-1', task: { id: 'task-1' } });
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.agent_team_start_member',
      params: { teamId: 'team-1', memberId: 'm1', taskId: 'task-1' },
    });
  });

  it('omits taskId when auto-picking and surfaces a blocked outcome', async () => {
    mockCall.mockResolvedValueOnce({ result: { kind: 'blocked', unmet: ['task-a'] } });
    const outcome = await agentTeamApi.startMember({ teamId: 'team-1', memberId: 'm1' });
    expect(outcome).toEqual({ kind: 'blocked', unmet: ['task-a'] });
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.agent_team_start_member',
      params: { teamId: 'team-1', memberId: 'm1' },
    });
  });

  it('throws when teamId or memberId is empty, without calling core', async () => {
    await expect(agentTeamApi.startMember({ teamId: '', memberId: 'm1' })).rejects.toThrow(
      'required'
    );
    await expect(agentTeamApi.startMember({ teamId: 'team-1', memberId: '' })).rejects.toThrow(
      'required'
    );
    expect(mockCall).not.toHaveBeenCalled();
  });
});
