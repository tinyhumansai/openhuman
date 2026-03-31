import { useCallback, useEffect, useState } from 'react';

import { callCoreRpc } from '../services/coreRpcClient';
import type { AIStatus } from '../store/aiSlice';
import { useAppSelector } from '../store/hooks';
import { aiListMemoryFiles, type GraphRelation, memoryGraphQuery } from '../utils/tauriCommands';

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

/** Derive entity-type counts from local graph relations. */
function entityCountsFromRelations(relations: GraphRelation[]): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const rel of relations) {
    const types = (rel.attrs?.entity_types ?? {}) as Record<string, string>;
    const subjectType = types.subject ?? 'entity';
    const objectType = types.object ?? 'entity';
    counts[subjectType] = (counts[subjectType] ?? 0) + 1;
    counts[objectType] = (counts[objectType] ?? 0) + 1;
  }
  return counts;
}

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
      const index = await callCoreRpc<Record<string, SessionEntry>>({
        method: 'ai.sessions_load_index',
      });
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
      const files = await aiListMemoryFiles('memory');
      setMemoryFiles(files.length);
    } catch {
      setMemoryFiles(null);
    }

    // Derive entity counts from local graph store
    try {
      const relations = await memoryGraphQuery();
      const counts = entityCountsFromRelations(relations);
      if (Object.keys(counts).length > 0) {
        setEntities(counts);
        setEntityError(false);
      } else {
        setEntities(null);
        setEntityError(false);
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

  return { sessions, memoryFiles, entities, entityError, aiStatus, isLoading, refetch: fetchStats };
}
