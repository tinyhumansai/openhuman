import { useCallback, useEffect, useRef, useState } from 'react';

import { callCoreRpc } from '../services/coreRpcClient';

export interface MemoryIngestionStatus {
  running: boolean;
  currentDocumentId?: string;
  currentTitle?: string;
  currentNamespace?: string;
  queueDepth: number;
  lastCompletedAt?: number;
  lastDocumentId?: string;
  lastSuccess?: boolean;
}

interface IngestionStatusEnvelope {
  running: boolean;
  current_document_id?: string;
  current_title?: string;
  current_namespace?: string;
  queue_depth: number;
  last_completed_at?: number;
  last_document_id?: string;
  last_success?: boolean;
}

const DEFAULT_POLL_MS = 4000;
const FAST_POLL_MS = 1500;

const EMPTY_STATUS: MemoryIngestionStatus = { running: false, queueDepth: 0 };

/**
 * Polls `openhuman.memory_ingestion_status`. Polls faster while a job is
 * running or queued so the UI reacts quickly when ingestion finishes;
 * relaxes to a slower cadence at idle.
 */
export function useMemoryIngestionStatus(): {
  status: MemoryIngestionStatus;
  loading: boolean;
  error: string | null;
  refresh: () => void;
} {
  const [status, setStatus] = useState<MemoryIngestionStatus>(EMPTY_STATUS);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const cancelledRef = useRef(false);

  const fetchOnce = useCallback(async () => {
    try {
      const env = await callCoreRpc<IngestionStatusEnvelope>({
        method: 'openhuman.memory_ingestion_status',
      });
      if (cancelledRef.current) return;
      setStatus({
        running: env.running,
        currentDocumentId: env.current_document_id,
        currentTitle: env.current_title,
        currentNamespace: env.current_namespace,
        queueDepth: env.queue_depth ?? 0,
        lastCompletedAt: env.last_completed_at,
        lastDocumentId: env.last_document_id,
        lastSuccess: env.last_success,
      });
      setError(null);
    } catch (err) {
      if (cancelledRef.current) return;
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (!cancelledRef.current) setLoading(false);
    }
  }, []);

  const statusRef = useRef(status);
  statusRef.current = status;

  useEffect(() => {
    cancelledRef.current = false;
    let timer: ReturnType<typeof setTimeout> | null = null;

    const tick = async () => {
      await fetchOnce();
      if (cancelledRef.current) return;
      const live = statusRef.current;
      const delay = live.running || live.queueDepth > 0 ? FAST_POLL_MS : DEFAULT_POLL_MS;
      timer = setTimeout(tick, delay);
    };

    void tick();

    return () => {
      cancelledRef.current = true;
      if (timer) clearTimeout(timer);
    };
  }, [fetchOnce]);

  return { status, loading, error, refresh: fetchOnce };
}
