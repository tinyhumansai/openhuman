import type { ApiResponse } from '../../types/api';
import type {
  PurgeRequestBody,
  PurgeResultData,
  SendMessageData,
  ThreadCreateData,
  ThreadMessagesData,
  ThreadsListData,
} from '../../types/thread';
import { apiClient } from '../apiClient';

export const threadApi = {
  /** GET /telegram/threads — list all threads for the authenticated user */
  getThreads: async (): Promise<ThreadsListData> => {
    const response = await apiClient.get<ApiResponse<ThreadsListData>>('/telegram/threads');
    return response.data;
  },

  /** POST /telegram/threads — create a new thread */
  createThread: async (chatId?: number): Promise<ThreadCreateData> => {
    const response = await apiClient.post<ApiResponse<ThreadCreateData>>(
      '/telegram/threads',
      chatId != null ? { chatId } : undefined
    );
    return response.data;
  },

  /** GET /telegram/threads/:threadId/messages — get messages for a thread */
  getThreadMessages: async (threadId: string): Promise<ThreadMessagesData> => {
    const response = await apiClient.get<ApiResponse<ThreadMessagesData>>(
      `/telegram/threads/${encodeURIComponent(threadId)}/messages`
    );
    return response.data;
  },

  /** POST /chat/autocomplete — send a user message to a thread and get the agent response */
  sendMessage: async (message: string, conversationId: string): Promise<SendMessageData> => {
    const response = await apiClient.post<ApiResponse<SendMessageData>>('/chat/autocomplete', {
      message,
      conversationId,
    });
    return response.data;
  },

  /** POST /telegram/purge — purge messages and/or threads */
  purge: async (body: PurgeRequestBody): Promise<PurgeResultData> => {
    const response = await apiClient.post<ApiResponse<PurgeResultData>>('/telegram/purge', body);
    return response.data;
  },
};
