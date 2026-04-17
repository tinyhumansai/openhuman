import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockCallCoreRpc = vi.fn();

vi.mock('../coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

describe('threadApi', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('loads threads from the threads RPC store', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      data: {
        threads: [
          {
            id: 'default-thread',
            title: 'Conversation',
            chatId: null,
            isActive: true,
            messageCount: 2,
            lastMessageAt: '2026-04-10T12:01:00Z',
            createdAt: '2026-04-10T12:00:00Z',
          },
        ],
        count: 1,
      },
    });

    const { threadApi } = await import('./threadApi');
    const result = await threadApi.getThreads();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.threads_list' });
    expect(result.count).toBe(1);
    expect(result.threads[0].id).toBe('default-thread');
  });

  it('appends a message via threads RPC', async () => {
    const message = {
      id: 'm1',
      content: 'hello',
      type: 'text',
      extraMetadata: {},
      sender: 'user' as const,
      createdAt: '2026-04-10T12:01:00Z',
    };
    mockCallCoreRpc.mockResolvedValueOnce({ data: message });

    const { threadApi } = await import('./threadApi');
    const result = await threadApi.appendMessage('default-thread', message);

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.threads_message_append',
      params: { thread_id: 'default-thread', message },
    });
    expect(result).toEqual(message);
  });
});
