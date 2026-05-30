import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../coreRpcClient';
import { todosApi, USER_TASKS_THREAD_ID } from './todosApi';

vi.mock('../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const mockCall = vi.mocked(callCoreRpc);

function snapshot(cards: unknown[], threadId: string | null = USER_TASKS_THREAD_ID) {
  return { threadId, cards, markdown: '' };
}

describe('todosApi', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('exposes a stable reserved personal board id', () => {
    expect(USER_TASKS_THREAD_ID).toBe('user-tasks');
  });

  it('list maps a snapshot into a TaskBoard and derives updatedAt from the latest card', async () => {
    mockCall.mockResolvedValueOnce(
      snapshot([
        { id: 'a', title: 'A', status: 'todo', order: 0, updatedAt: '2026-01-01T00:00:00Z' },
        { id: 'b', title: 'B', status: 'done', order: 1, updatedAt: '2026-02-01T00:00:00Z' },
      ])
    );
    const board = await todosApi.list(USER_TASKS_THREAD_ID);
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.todos_list',
      params: { thread_id: USER_TASKS_THREAD_ID },
    });
    expect(board.threadId).toBe(USER_TASKS_THREAD_ID);
    expect(board.cards).toHaveLength(2);
    expect(board.updatedAt).toBe('2026-02-01T00:00:00Z');
  });

  it('add omits undefined fields but preserves explicit nulls', async () => {
    mockCall.mockResolvedValueOnce(snapshot([]));
    await todosApi.add({
      threadId: USER_TASKS_THREAD_ID,
      content: 'Buy milk',
      status: 'todo',
      objective: null,
    });
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.todos_add',
      params: {
        thread_id: USER_TASKS_THREAD_ID,
        content: 'Buy milk',
        status: 'todo',
        objective: null,
      },
    });
    // `notes` was undefined → pruned from the wire params.
    const params = mockCall.mock.calls[0][0].params as Record<string, unknown>;
    expect('notes' in params).toBe(false);
  });

  it('edit forwards camelCase patch fields and a clearing null approvalMode', async () => {
    mockCall.mockResolvedValueOnce(snapshot([]));
    await todosApi.edit({
      threadId: 't-1',
      id: 'card-1',
      content: 'New title',
      approvalMode: null,
      allowedTools: ['todo'],
    });
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.todos_edit',
      params: {
        thread_id: 't-1',
        id: 'card-1',
        content: 'New title',
        approvalMode: null,
        allowedTools: ['todo'],
      },
    });
  });

  it('updateStatus and remove call the matching RPC methods', async () => {
    mockCall.mockResolvedValueOnce(snapshot([]));
    await todosApi.updateStatus('t-1', 'card-1', 'done');
    expect(mockCall).toHaveBeenLastCalledWith({
      method: 'openhuman.todos_update_status',
      params: { thread_id: 't-1', id: 'card-1', status: 'done' },
    });

    mockCall.mockResolvedValueOnce(snapshot([]));
    await todosApi.remove('t-1', 'card-1');
    expect(mockCall).toHaveBeenLastCalledWith({
      method: 'openhuman.todos_remove',
      params: { thread_id: 't-1', id: 'card-1' },
    });
  });

  it('falls back to the request thread id when the snapshot omits one', async () => {
    mockCall.mockResolvedValueOnce(snapshot([], null));
    const board = await todosApi.list('t-xyz');
    expect(board.threadId).toBe('t-xyz');
  });
});
