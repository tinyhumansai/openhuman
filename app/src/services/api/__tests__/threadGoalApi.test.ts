import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockCallCoreRpc = vi.fn();

vi.mock('../../coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

const { threadGoalApi } = await import('../threadGoalApi');

const goal = {
  threadId: 't1',
  goalId: 'g-uuid',
  objective: 'ship it',
  status: 'active' as const,
  tokenBudget: 1000,
  tokensUsed: 100,
  timeUsedSeconds: 5,
  createdAtMs: 1,
  updatedAtMs: 2,
  continuationSuppressed: false,
};

describe('threadGoalApi', () => {
  beforeEach(() => mockCallCoreRpc.mockReset());

  it('get returns the bare goal envelope', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ goal });
    const out = await threadGoalApi.get('t1');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.thread_goals_get',
      params: { thread_id: 't1' },
    });
    expect(out?.objective).toBe('ship it');
  });

  it('get returns null when the thread has no goal', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ goal: null });
    expect(await threadGoalApi.get('t1')).toBeNull();
  });

  it('set unwraps the { result, logs } envelope and forwards the budget', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: { goal }, logs: ['set'] });
    const out = await threadGoalApi.set('t1', 'ship it', 1000);
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.thread_goals_set',
      params: { thread_id: 't1', objective: 'ship it', token_budget: 1000 },
    });
    expect(out?.goalId).toBe('g-uuid');
  });

  it('set omits token_budget when not provided', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: { goal }, logs: [] });
    await threadGoalApi.set('t1', 'ship it');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.thread_goals_set',
      params: { thread_id: 't1', objective: 'ship it' },
    });
  });

  it('complete / pause / resume hit the right methods', async () => {
    mockCallCoreRpc.mockResolvedValue({ result: { goal }, logs: [] });
    await threadGoalApi.complete('t1');
    await threadGoalApi.pause('t1');
    await threadGoalApi.resume('t1');
    const methods = mockCallCoreRpc.mock.calls.map(c => (c[0] as { method: string }).method);
    expect(methods).toEqual([
      'openhuman.thread_goals_complete',
      'openhuman.thread_goals_pause',
      'openhuman.thread_goals_resume',
    ]);
  });

  it('clear returns the removed flag', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: { removed: true }, logs: ['cleared'] });
    expect(await threadGoalApi.clear('t1')).toBe(true);
    mockCallCoreRpc.mockResolvedValueOnce({ removed: false });
    expect(await threadGoalApi.clear('t1')).toBe(false);
  });
});
