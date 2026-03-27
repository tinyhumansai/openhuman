import { useCallback, useEffect, useState } from 'react';

import { MOCK_ACTIONABLE_ITEMS } from '../components/intelligence/mockData';
import { type ConnectedTool, intelligenceApi } from '../services/intelligenceApi';
import type { ActionableItem, ActionableItemStatus, ChatMessage } from '../types/intelligence';
import {
  transformBackendItemsToFrontend,
  transformBackendMessagesToFrontend,
} from '../utils/intelligenceTransforms';

/**
 * Fallback implementation of Intelligence API hooks without React Query
 * Used when React Query is not available
 */

interface UseActionableItemsResult {
  data: ActionableItem[] | undefined;
  loading: boolean;
  error: string | null;
  refetch: () => Promise<void>;
}

/**
 * Hook for fetching actionable items (fallback version)
 * TODO: Remove MOCK_ACTIONABLE_ITEMS once backend APIs are ready
 */
export const useActionableItems = (options?: {
  refetchInterval?: number;
  enabled?: boolean;
}): UseActionableItemsResult => {
  const [data, setData] = useState<ActionableItem[] | undefined>(MOCK_ACTIONABLE_ITEMS);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const fetchItems = useCallback(async () => {
    if (options?.enabled === false) return;

    try {
      setLoading(true);
      setError(null);
      const backendItems = await intelligenceApi.getActionableItems();
      const items = transformBackendItemsToFrontend(backendItems);
      setData(items.length > 0 ? items : MOCK_ACTIONABLE_ITEMS);
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : 'Failed to fetch items';
      setError(errorMessage);
      console.error('Failed to fetch actionable items:', err);
      // TODO: Replace with actual data
      setData(MOCK_ACTIONABLE_ITEMS);
    } finally {
      setLoading(false);
    }
  }, [options?.enabled]);

  // Initial fetch
  useEffect(() => {
    fetchItems();
  }, [fetchItems]);

  // Set up refetch interval
  useEffect(() => {
    if (options?.refetchInterval) {
      const interval = setInterval(fetchItems, options.refetchInterval);
      return () => clearInterval(interval);
    }
  }, [options?.refetchInterval, fetchItems]);

  return { data, loading, error, refetch: fetchItems };
};

interface UseUpdateActionableItemResult {
  mutateAsync: (variables: {
    itemId: string;
    status: ActionableItemStatus;
  }) => Promise<{ itemId: string; status: ActionableItemStatus; updatedAt: Date }>;
  loading: boolean;
  error: string | null;
}

/**
 * Hook for updating actionable item status (fallback version)
 */
export const useUpdateActionableItem = (): UseUpdateActionableItemResult => {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const mutateAsync = useCallback(
    async (variables: { itemId: string; status: ActionableItemStatus }) => {
      try {
        setLoading(true);
        setError(null);
        await intelligenceApi.updateItemStatus(variables.itemId, variables.status);
        return { ...variables, updatedAt: new Date() };
      } catch (err) {
        const errorMessage = err instanceof Error ? err.message : 'Failed to update item';
        setError(errorMessage);
        throw err;
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
 * Hook for snoozing actionable item (fallback version)
 */
export const useSnoozeActionableItem = (): UseSnoozeActionableItemResult => {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const mutateAsync = useCallback(async (variables: { itemId: string; snoozeUntil: Date }) => {
    try {
      setLoading(true);
      setError(null);
      await intelligenceApi.snoozeItem(variables.itemId, variables.snoozeUntil);
      return { ...variables, updatedAt: new Date() };
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : 'Failed to snooze item';
      setError(errorMessage);
      throw err;
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
 * Hook for creating or getting chat session (fallback version)
 */
export const useChatSession = (itemId: string | null): UseChatSessionResult => {
  const [data, setData] = useState<{ threadId: string; messages: ChatMessage[] } | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!itemId) return;

    const fetchSession = async () => {
      try {
        setLoading(true);
        setError(null);
        const response = await intelligenceApi.getOrCreateThread(itemId);
        setData({
          threadId: response.threadId,
          messages: transformBackendMessagesToFrontend(response.messages),
        });
      } catch (err) {
        const errorMessage = err instanceof Error ? err.message : 'Failed to create session';
        setError(errorMessage);
        console.error('Failed to create chat session:', err);
      } finally {
        setLoading(false);
      }
    };

    fetchSession();
  }, [itemId]);

  return { data, loading, error };
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
 * Hook for executing tasks (fallback version)
 */
export const useExecuteTask = (): UseExecuteTaskResult => {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const mutateAsync = useCallback(
    async (variables: { itemId: string; connectedTools: ConnectedTool[] }) => {
      try {
        setLoading(true);
        setError(null);
        const result = await intelligenceApi.executeTask(
          variables.itemId,
          variables.connectedTools
        );
        return result;
      } catch (err) {
        const errorMessage = err instanceof Error ? err.message : 'Failed to execute task';
        setError(errorMessage);
        throw err;
      } finally {
        setLoading(false);
      }
    },
    []
  );

  return { mutateAsync, loading, error };
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
