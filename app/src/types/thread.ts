export interface Thread {
  id: string;
  title: string;
  chatId: number | null;
  isActive: boolean;
  messageCount: number;
  lastMessageAt: string;
  createdAt: string;
  parentThreadId?: string;
  labels: string[];
}

export interface ThreadMessage {
  id: string;
  content: string;
  type: string;
  extraMetadata: Record<string, unknown>;
  sender: 'user' | 'agent';
  createdAt: string;
}

export interface ThreadsListData {
  threads: Thread[];
  count: number;
}

export interface ThreadMessagesData {
  messages: ThreadMessage[];
  count: number;
}

export interface ThreadCreateData {
  id: string;
}

export interface ThreadDeleteData {
  deleted: boolean;
}

/** Response from POST /chat/sendMessage — send user message and get agent reply */
export interface SendMessageResponseData {
  // Optional: backend can return empty {} or e.g. { messageId: string }
  [key: string]: unknown;
}

export interface PurgeRequestBody {
  messages: boolean;
  agentThreads: boolean;
  deleteEverything: boolean;
  deleteFrom?: string;
  deleteTo?: string;
}

export interface PurgeResultData {
  messagesDeleted: number;
  agentThreadsDeleted: number;
  agentMessagesDeleted: number;
}
