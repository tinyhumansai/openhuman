import debug from 'debug';

import type {
  PurgeResultData,
  Thread,
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

const generateTitleLog = debug('threadApi.generateTitleIfNeeded');

export const threadApi = {
  createNewThread: async (): Promise<Thread> => {
    const response = await callCoreRpc<Envelope<Thread>>({
      method: 'openhuman.threads_create_new',
    });
    return unwrapEnvelope(response);
  },

  getThreads: async (): Promise<ThreadsListData> => {
    const response = await callCoreRpc<Envelope<ThreadsListData>>({
      method: 'openhuman.threads_list',
    });
    return unwrapEnvelope(response);
  },

  getThreadMessages: async (threadId: string): Promise<ThreadMessagesData> => {
    const response = await callCoreRpc<Envelope<ThreadMessagesData>>({
      method: 'openhuman.threads_messages_list',
      params: { thread_id: threadId },
    });
    return unwrapEnvelope(response);
  },

  appendMessage: async (threadId: string, message: ThreadMessage): Promise<ThreadMessage> => {
    const response = await callCoreRpc<Envelope<ThreadMessage>>({
      method: 'openhuman.threads_message_append',
      params: { thread_id: threadId, message },
    });
    return unwrapEnvelope(response);
  },

  generateTitleIfNeeded: async (threadId: string, assistantMessage?: string): Promise<Thread> => {
    generateTitleLog('enter threadId=%s assistantMessage=%o', threadId, assistantMessage);
    try {
      const response = await callCoreRpc<Envelope<Thread>>({
        method: 'openhuman.threads_generate_title',
        params: { thread_id: threadId, assistant_message: assistantMessage },
      });
      const thread = unwrapEnvelope(response);
      generateTitleLog('success threadId=%s response=%o thread=%o', threadId, response, thread);
      return thread;
    } catch (error) {
      generateTitleLog(
        'error threadId=%s assistantMessage=%o error=%O',
        threadId,
        assistantMessage,
        error
      );
      throw error;
    }
  },

  updateMessage: async (
    threadId: string,
    messageId: string,
    extraMetadata: Record<string, unknown>
  ): Promise<ThreadMessage> => {
    const response = await callCoreRpc<Envelope<ThreadMessage>>({
      method: 'openhuman.threads_message_update',
      params: { thread_id: threadId, message_id: messageId, extra_metadata: extraMetadata },
    });
    return unwrapEnvelope(response);
  },

  deleteThread: async (threadId: string): Promise<ThreadDeleteData> => {
    const response = await callCoreRpc<Envelope<ThreadDeleteData>>({
      method: 'openhuman.threads_delete',
      params: { thread_id: threadId, deleted_at: new Date().toISOString() },
    });
    return unwrapEnvelope(response);
  },

  purge: async (): Promise<PurgeResultData> => {
    const response = await callCoreRpc<Envelope<PurgeResultData>>({
      method: 'openhuman.threads_purge',
    });
    return unwrapEnvelope(response);
  },
};
