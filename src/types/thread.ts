export interface Thread {
  id: string;
  title: string;
  chatId: number | null;
  isActive: boolean;
  messageCount: number;
  lastMessageAt: string;
  createdAt: string;
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

export interface SendMessageData {
  suggestions: Array<{ text: string; confidence: number }>;
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
