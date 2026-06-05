import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockCallCoreRpc = vi.fn();

vi.mock('../../coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

const { todosApi } = await import('../todosApi');

describe('todosApi.setSessionThread', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('links a card to its session thread via the RPC and returns the board', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      threadId: 'user-tasks',
      cards: [
        { id: 'c1', title: 'T', status: 'todo', order: 0, updatedAt: '2026-01-01T00:00:00Z' },
      ],
    });

    const board = await todosApi.setSessionThread('user-tasks', 'c1', 'task-abc');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.todos_set_session_thread',
      params: { thread_id: 'user-tasks', id: 'c1', sessionThreadId: 'task-abc' },
    });
    expect(board.threadId).toBe('user-tasks');
    expect(board.cards).toHaveLength(1);
  });

  it('passes null through to clear the link', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ threadId: 'user-tasks', cards: [] });

    await todosApi.setSessionThread('user-tasks', 'c1', null);

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.todos_set_session_thread',
      params: { thread_id: 'user-tasks', id: 'c1', sessionThreadId: null },
    });
  });
});
