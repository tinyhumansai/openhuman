export interface RespondQueueItem {
  id: string;
  provider: string;
  accountId: string;
  eventKind: string;
  entityId: string;
  threadId?: string;
  title?: string;
  snippet?: string;
  senderName?: string;
  senderHandle?: string;
  timestamp: string;
  deepLink?: string;
  requiresAttention: boolean;
  status: string;
}

export interface RespondQueueList {
  items: RespondQueueItem[];
  count: number;
}
