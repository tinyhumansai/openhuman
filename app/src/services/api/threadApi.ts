import type {
  PurgeResultData,
  Thread,
  ThreadCreateData,
  ThreadDeleteData,
  ThreadMessage,
  ThreadMessagesData,
  ThreadsListData,
} from '../../types/thread';
import { callCoreRpc } from '../coreRpcClient';

interface Envelope<T> {
  data?: T;
}

function unwrapEnvelope<T>(response: Envelope<T> | T): T {
  if (response && typeof response === 'object' && 'data' in response) {
    return (response as Envelope<T>).data as T;
  }
  return response as T;
}

export const threadApi = {
  createNewThread: async (): Promise<Thread> => {
    const response = await callCoreRpc<Envelope<Thread>>({
      method: 'openhuman.memory_thread_create_new',
    });
    return unwrapEnvelope(response);
  },

  getThreads: async (): Promise<ThreadsListData> => {
    const response = await callCoreRpc<Envelope<ThreadsListData>>({
      method: 'openhuman.memory_threads_list',
    });
    return unwrapEnvelope(response);
  },

  createThread: async (input: {
    id: string;
    title: string;
    createdAt: string;
  }): Promise<ThreadCreateData> => {
    const response = await callCoreRpc<Envelope<Thread>>({
      method: 'openhuman.memory_thread_upsert',
      params: { id: input.id, title: input.title, created_at: input.createdAt },
    });
    const thread = unwrapEnvelope(response);
    return { id: thread.id };
  },

  getThreadMessages: async (threadId: string): Promise<ThreadMessagesData> => {
    const response = await callCoreRpc<Envelope<ThreadMessagesData>>({
      method: 'openhuman.memory_messages_list',
      params: { thread_id: threadId },
    });
    return unwrapEnvelope(response);
  },

  appendMessage: async (threadId: string, message: ThreadMessage): Promise<ThreadMessage> => {
    const response = await callCoreRpc<Envelope<ThreadMessage>>({
      method: 'openhuman.memory_message_append',
      params: { thread_id: threadId, message },
    });
    return unwrapEnvelope(response);
  },

  updateMessage: async (
    threadId: string,
    messageId: string,
    extraMetadata: Record<string, unknown>
  ): Promise<ThreadMessage> => {
    const response = await callCoreRpc<Envelope<ThreadMessage>>({
      method: 'openhuman.memory_message_update',
      params: { thread_id: threadId, message_id: messageId, extra_metadata: extraMetadata },
    });
    return unwrapEnvelope(response);
  },

  deleteThread: async (threadId: string): Promise<ThreadDeleteData> => {
    const response = await callCoreRpc<Envelope<ThreadDeleteData>>({
      method: 'openhuman.memory_thread_delete',
      params: { thread_id: threadId, deleted_at: new Date().toISOString() },
    });
    return unwrapEnvelope(response);
  },

  purge: async (): Promise<PurgeResultData> => {
    const response = await callCoreRpc<Envelope<PurgeResultData>>({
      method: 'openhuman.memory_threads_purge',
    });
    return unwrapEnvelope(response);
  },
};
