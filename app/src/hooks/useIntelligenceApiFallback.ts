import { useCallback, useState } from 'react';

import type { ActionableItemStatus, ChatMessage } from '../types/intelligence';

interface ConnectedTool {
  name: string;
  description: string;
  parameters: Record<string, unknown>;
  skillId: string;
  enabled: boolean;
}

/**
 * Local-only implementations of Intelligence action hooks.
 * Items come from the local conscious memory layer — actions are applied in-memory.
 */

interface UseUpdateActionableItemResult {
  mutateAsync: (variables: {
    itemId: string;
    status: ActionableItemStatus;
  }) => Promise<{ itemId: string; status: ActionableItemStatus; updatedAt: Date }>;
  loading: boolean;
  error: string | null;
}

/**
 * Hook for updating actionable item status (local-only).
 */
export const useUpdateActionableItem = (): UseUpdateActionableItemResult => {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const mutateAsync = useCallback(
    async (variables: { itemId: string; status: ActionableItemStatus }) => {
      setLoading(true);
      setError(null);
      try {
        // Items are managed locally; just acknowledge the status change.
        return { ...variables, updatedAt: new Date() };
      } finally {
        setLoading(false);
      }
    },
    []
  );

  return { mutateAsync, loading, error };
};

interface UseSnoozeActionableItemResult {
  mutateAsync: (variables: {
    itemId: string;
    snoozeUntil: Date;
  }) => Promise<{ itemId: string; snoozeUntil: Date; updatedAt: Date }>;
  loading: boolean;
  error: string | null;
}

/**
 * Hook for snoozing actionable item (local-only).
 */
export const useSnoozeActionableItem = (): UseSnoozeActionableItemResult => {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const mutateAsync = useCallback(async (variables: { itemId: string; snoozeUntil: Date }) => {
    setLoading(true);
    setError(null);
    try {
      return { ...variables, updatedAt: new Date() };
    } finally {
      setLoading(false);
    }
  }, []);

  return { mutateAsync, loading, error };
};

interface UseChatSessionResult {
  data: { threadId: string; messages: ChatMessage[] } | null;
  loading: boolean;
  error: string | null;
}

/**
 * Chat session stub (local-only — no remote thread API).
 */
export const useChatSession = (_itemId: string | null): UseChatSessionResult => {
  return { data: null, loading: false, error: null };
};

interface UseExecuteTaskResult {
  mutateAsync: (variables: {
    itemId: string;
    connectedTools: ConnectedTool[];
  }) => Promise<{ executionId: string; sessionId: string; status: string }>;
  loading: boolean;
  error: string | null;
}

/**
 * Task execution stub (local-only — no remote execution API).
 */
export const useExecuteTask = (): UseExecuteTaskResult => {
  const mutateAsync = useCallback(
    async (_variables: { itemId: string; connectedTools: ConnectedTool[] }) => {
      return { executionId: '', sessionId: '', status: 'unsupported' };
    },
    []
  );

  return { mutateAsync, loading: false, error: null };
};

// Export query key utilities for consistency
export const intelligenceKeys = {
  all: ['intelligence'] as const,
  items: () => [...intelligenceKeys.all, 'items'] as const,
  item: (id: string) => [...intelligenceKeys.all, 'item', id] as const,
  thread: (itemId: string) => [...intelligenceKeys.all, 'thread', itemId] as const,
  messages: (threadId: string) => [...intelligenceKeys.all, 'messages', threadId] as const,
  execution: (executionId: string) => [...intelligenceKeys.all, 'execution', executionId] as const,
};
