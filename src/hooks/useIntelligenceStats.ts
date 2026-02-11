import { invoke } from '@tauri-apps/api/core';
import { useCallback, useEffect, useState } from 'react';

import { apiClient } from '../services/apiClient';
import type { AIStatus } from '../store/aiSlice';
import { useAppSelector } from '../store/hooks';

interface SessionEntry {
  sessionId: string;
  updatedAt: number;
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
  compactionCount: number;
  memoryFlushAt?: number;
}

interface SessionStats {
  total: number;
  totalTokens: number;
  compactions: number;
  memoryFlushes: number;
}

export interface IntelligenceStats {
  sessions: SessionStats | null;
  memoryFiles: number | null;
  entities: Record<string, number> | null;
  entityError: boolean;
  aiStatus: AIStatus;
  isLoading: boolean;
  refetch: () => void;
}

const ENTITY_TYPES = ['contact', 'chat', 'message', 'wallet', 'token', 'transaction'];

export function useIntelligenceStats(): IntelligenceStats {
  const aiStatus = useAppSelector(state => state.ai.status);
  const [sessions, setSessions] = useState<SessionStats | null>(null);
  const [memoryFiles, setMemoryFiles] = useState<number | null>(null);
  const [entities, setEntities] = useState<Record<string, number> | null>(null);
  const [entityError, setEntityError] = useState(false);
  const [isLoading, setIsLoading] = useState(true);

  const fetchStats = useCallback(async () => {
    setIsLoading(true);

    // Fetch local stats (Tauri invoke)
    try {
      const index = await invoke<Record<string, SessionEntry>>('ai_sessions_load_index');
      const entries = Object.values(index);
      setSessions({
        total: entries.length,
        totalTokens: entries.reduce((sum, e) => sum + (e.totalTokens || 0), 0),
        compactions: entries.reduce((sum, e) => sum + (e.compactionCount || 0), 0),
        memoryFlushes: entries.filter(e => e.memoryFlushAt).length,
      });
    } catch {
      setSessions(null);
    }

    try {
      const files = await invoke<string[]>('ai_list_memory_files', { relativeDir: 'memory' });
      setMemoryFiles(files.length);
    } catch {
      setMemoryFiles(null);
    }

    // Fetch entity counts from backend API (graceful degradation)
    try {
      const counts: Record<string, number> = {};
      const results = await Promise.allSettled(
        ENTITY_TYPES.map(async type => {
          const resp = await apiClient.get<{ count?: number; total?: number; data?: unknown[] }>(
            `/api/entity-graph/entities?type=${type}&limit=1`
          );
          return { type, count: resp.count ?? resp.total ?? (resp.data ? resp.data.length : 0) };
        })
      );

      let anySuccess = false;
      for (const result of results) {
        if (result.status === 'fulfilled') {
          counts[result.value.type] = result.value.count;
          anySuccess = true;
        }
      }

      if (anySuccess) {
        setEntities(counts);
        setEntityError(false);
      } else {
        setEntities(null);
        setEntityError(true);
      }
    } catch {
      setEntities(null);
      setEntityError(true);
    }

    setIsLoading(false);
  }, []);

  useEffect(() => {
    fetchStats();
  }, [fetchStats, aiStatus]);

  return {
    sessions,
    memoryFiles,
    entities,
    entityError,
    aiStatus,
    isLoading,
    refetch: fetchStats,
  };
}
