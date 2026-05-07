import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { threadApi } from '../services/api/threadApi';
import type { ThreadMessagesData, ThreadsListData } from '../types/thread';

const log = debug('hooks:threadQueries');

export interface ThreadQueryState<T> {
  data: T | null;
  loading: boolean;
  error: Error | null;
  isRefetching: boolean;
  refetch: () => Promise<T | undefined>;
}

function normalizeError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}

function useThreadQuery<T>(
  queryName: string,
  load: () => Promise<T>,
  enabled = true,
  queryKey = queryName
): ThreadQueryState<T> {
  const [data, setData] = useState<T | null>(null);
  const [loading, setLoading] = useState(enabled);
  const [error, setError] = useState<Error | null>(null);
  const [isRefetching, setIsRefetching] = useState(false);
  const requestIdRef = useRef(0);
  const dataRef = useRef<T | null>(null);
  const queryKeyRef = useRef(queryKey);

  const execute = useCallback(
    async (reason: 'initial' | 'refetch'): Promise<T | undefined> => {
      if (!enabled) {
        log('%s skip disabled reason=%s', queryName, reason);
        return undefined;
      }

      const requestId = requestIdRef.current + 1;
      requestIdRef.current = requestId;
      const hasData = dataRef.current !== null;
      log('%s start requestId=%d reason=%s hasData=%s', queryName, requestId, reason, hasData);

      setError(null);
      if (hasData || reason === 'refetch') {
        setIsRefetching(true);
      } else {
        setLoading(true);
      }

      try {
        const nextData = await load();
        if (requestIdRef.current !== requestId) {
          log('%s ignore stale success requestId=%d', queryName, requestId);
          return nextData;
        }
        dataRef.current = nextData;
        setData(nextData);
        log('%s success requestId=%d', queryName, requestId);
        return nextData;
      } catch (caught) {
        const nextError = normalizeError(caught);
        if (requestIdRef.current !== requestId) {
          log('%s ignore stale error requestId=%d error=%o', queryName, requestId, nextError);
          return undefined;
        }
        setError(nextError);
        log('%s error requestId=%d error=%o', queryName, requestId, nextError);
        return undefined;
      } finally {
        if (requestIdRef.current === requestId) {
          setLoading(false);
          setIsRefetching(false);
        }
      }
    },
    [enabled, load, queryName]
  );

  useEffect(() => {
    if (queryKeyRef.current !== queryKey) {
      requestIdRef.current += 1;
      queryKeyRef.current = queryKey;
      dataRef.current = null;
      setData(null);
      setError(null);
      setIsRefetching(false);
    }

    if (!enabled) {
      requestIdRef.current += 1;
      dataRef.current = null;
      setData(null);
      setError(null);
      setLoading(false);
      setIsRefetching(false);
      return;
    }
    void execute('initial');
  }, [enabled, execute, queryKey]);

  useEffect(
    () => () => {
      requestIdRef.current += 1;
    },
    []
  );

  const refetch = useCallback(() => execute('refetch'), [execute]);

  return { data, loading, error, isRefetching, refetch };
}

export function useThreads(): ThreadQueryState<ThreadsListData> {
  const load = useCallback(() => threadApi.getThreads(), []);
  return useThreadQuery('threads.list', load);
}

export function useThreadMessages(threadId?: string | null): ThreadQueryState<ThreadMessagesData> {
  const normalizedThreadId = threadId?.trim() || null;
  const load = useCallback(
    () => threadApi.getThreadMessages(normalizedThreadId ?? ''),
    [normalizedThreadId]
  );
  return useThreadQuery(
    'threads.messages',
    load,
    normalizedThreadId !== null,
    `threads.messages:${normalizedThreadId ?? 'disabled'}`
  );
}
