/**
 * Three-pane memory_tree browser.
 *
 *   ┌──────────────┬──────────────────┬─────────────────────────┐
 *   │  NAVIGATOR   │  RESULT LIST     │  CHUNK DETAIL           │
 *   │  280px       │  380px           │  flex                   │
 *   └──────────────┴──────────────────┴─────────────────────────┘
 *
 * Replaces the legacy UnifiedMemory-driven workspace. Auto-selects the
 * most recent admitted chunk on mount; renders an empty placeholder when
 * the user has no chunks yet. Talks to the real `openhuman.memory_tree_*`
 * JSON-RPC surface via the `utils/tauriCommands/memoryTree` wrappers.
 */
import { useCallback, useEffect, useMemo, useState } from 'react';

import type { ToastNotification } from '../../types/intelligence';
import {
  type Chunk,
  type ChunkFilter,
  type EntityRef,
  memoryTreeListChunks,
  memoryTreeListSources,
  memoryTreeTopEntities,
  type Source,
} from '../../utils/tauriCommands';
import './memory-workspace.css';
import { MemoryChunkDetail } from './MemoryChunkDetail';
import { MemoryEmptyPlaceholder } from './MemoryEmptyPlaceholder';
import { MemoryNavigator, type NavigatorSelection } from './MemoryNavigator';
import { MemoryResultList } from './MemoryResultList';

interface MemoryWorkspaceProps {
  // Kept for backward compat with MemoryDataPanel — surfaced toast hook even
  // though the new browser doesn't trigger any side-effecting flows yet.
  // Call sites (e.g. settings/MemoryDataPanel) keep passing it; we silence
  // the unused-prop warning intentionally.
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  onToast?: (toast: Omit<ToastNotification, 'id'>) => void;
}

const MEDIA_QUERY = '(max-width: 1100px)';

function useIsCompact(): boolean {
  const getMatch = () =>
    typeof window !== 'undefined' && typeof window.matchMedia === 'function'
      ? window.matchMedia(MEDIA_QUERY).matches
      : false;
  const [isCompact, setIsCompact] = useState<boolean>(getMatch);
  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return;
    const mql = window.matchMedia(MEDIA_QUERY);
    const handler = (e: MediaQueryListEvent) => setIsCompact(e.matches);
    mql.addEventListener?.('change', handler);
    return () => {
      mql.removeEventListener?.('change', handler);
    };
  }, []);
  return isCompact;
}

export function MemoryWorkspace({ onToast: _onToast }: MemoryWorkspaceProps) {
  const [allChunks, setAllChunks] = useState<Chunk[]>([]);
  const [sources, setSources] = useState<Source[]>([]);
  const [topPeople, setTopPeople] = useState<EntityRef[]>([]);
  const [topTopics, setTopTopics] = useState<EntityRef[]>([]);
  const [selectedChunkId, setSelectedChunkId] = useState<string | null>(null);
  const [selection, setSelection] = useState<NavigatorSelection>({ sourceIds: [], entityIds: [] });
  const [searchQuery, setSearchQuery] = useState('');
  const [showDetailOnCompact, setShowDetailOnCompact] = useState(false);

  const isCompact = useIsCompact();

  // Initial data load. Fetches the full chunk set + navigator metadata
  // in parallel — mocks resolve synchronously, real RPCs will fan out.
  useEffect(() => {
    console.debug('[ui-flow][memory-workspace] initial load entry');
    let cancelled = false;
    void Promise.all([
      memoryTreeListChunks({ limit: 500 }),
      memoryTreeListSources(),
      memoryTreeTopEntities('person', 12),
      // Topics: union of technology/product/event types — fetch a wider list
      // then bucket below.
      memoryTreeTopEntities(undefined, 40),
    ]).then(([chunkResult, srcs, people, anyEntities]) => {
      if (cancelled) return;
      const topicKinds = new Set(['technology', 'product', 'event']);
      const topics = anyEntities.filter(e => topicKinds.has(e.kind)).slice(0, 12);

      setAllChunks(chunkResult.chunks);
      setSources(srcs);
      setTopPeople(people);
      setTopTopics(topics);

      // Auto-select most recent admitted chunk (or fall back to most recent).
      const sorted = [...chunkResult.chunks].sort((a, b) => b.timestamp_ms - a.timestamp_ms);
      const admitted = sorted.find(c => c.lifecycle_status === 'admitted');
      const seed = admitted ?? sorted[0];
      if (seed) {
        console.debug('[ui-flow][memory-workspace] auto-select seed chunk', seed.id);
        setSelectedChunkId(seed.id);
      } else {
        console.debug('[ui-flow][memory-workspace] no chunks — empty state');
      }
    });
    return () => {
      cancelled = true;
    };
  }, []);

  // Filter the full chunk set against the active navigator selection +
  // search query. We do this client-side because (a) for ~30 mocked
  // chunks the cost is negligible, and (b) the same shape will work
  // when the call is upgraded to a backend listChunks(filter) call.
  const filteredChunks = useMemo<Chunk[]>(() => {
    const filter: ChunkFilter = {
      source_ids: selection.sourceIds.length > 0 ? selection.sourceIds : undefined,
      entity_ids: selection.entityIds.length > 0 ? selection.entityIds : undefined,
      query: searchQuery.trim() || undefined,
    };
    return allChunks.filter(c => {
      if (filter.source_ids && !filter.source_ids.includes(c.source_id)) return false;
      if (filter.entity_ids) {
        const hit = filter.entity_ids.some(id => c.tags.includes(id));
        if (!hit) return false;
      }
      if (filter.query) {
        const needle = filter.query.toLowerCase();
        const hay = `${c.content_preview ?? ''} ${c.tags.join(' ')}`.toLowerCase();
        if (!hay.includes(needle)) return false;
      }
      return true;
    });
  }, [allChunks, selection, searchQuery]);

  // Keep the active selection valid as filters change. If the currently
  // selected chunk was filtered out, fall back to the top of the list.
  useEffect(() => {
    if (filteredChunks.length === 0) return;
    const stillVisible = filteredChunks.some(c => c.id === selectedChunkId);
    if (!stillVisible) {
      const sorted = [...filteredChunks].sort((a, b) => b.timestamp_ms - a.timestamp_ms);
      console.debug(
        '[ui-flow][memory-workspace] selection invalidated, reseeding to',
        sorted[0]?.id
      );
      setSelectedChunkId(sorted[0]?.id ?? null);
    }
  }, [filteredChunks, selectedChunkId]);

  const selectedChunk = useMemo(
    () => allChunks.find(c => c.id === selectedChunkId) ?? null,
    [allChunks, selectedChunkId]
  );

  const handleSelectChunk = useCallback((id: string) => {
    console.debug('[ui-flow][memory-workspace] select chunk', id);
    setSelectedChunkId(id);
    setShowDetailOnCompact(true);
  }, []);

  const handleSelectionChange = useCallback((next: NavigatorSelection) => {
    console.debug(
      '[ui-flow][memory-workspace] navigator selection change',
      'sources=',
      next.sourceIds.length,
      'entities=',
      next.entityIds.length
    );
    setSelection(next);
    setShowDetailOnCompact(false);
  }, []);

  const handleSearchChange = useCallback((q: string) => {
    setSearchQuery(q);
    setShowDetailOnCompact(false);
  }, []);

  const handleSelectEntity = useCallback((entity: EntityRef) => {
    const tag = `${entity.kind}/${entity.surface.replace(/\s+/g, '-')}`;
    console.debug('[ui-flow][memory-workspace] mentioned-entity click → activate lens', tag);
    setSelection(prev => {
      if (prev.entityIds.includes(tag)) return prev;
      return { ...prev, entityIds: [...prev.entityIds, tag] };
    });
    setShowDetailOnCompact(false);
  }, []);

  // Empty-state: brand-new user with zero chunks total.
  const isEmpty = allChunks.length === 0;

  const gridClassName = `memory-workspace-grid${
    isCompact && showDetailOnCompact ? ' mw-show-detail' : ''
  }`;

  return (
    <section className="memory-workspace-root" data-testid="memory-workspace">
      <div className={gridClassName}>
        <MemoryNavigator
          chunks={allChunks}
          sources={isEmpty ? [] : sources}
          topPeople={isEmpty ? [] : topPeople}
          topTopics={isEmpty ? [] : topTopics}
          selection={selection}
          onSelectionChange={handleSelectionChange}
          searchQuery={searchQuery}
          onSearchChange={handleSearchChange}
        />

        <MemoryResultList
          chunks={filteredChunks}
          selectedChunkId={selectedChunkId}
          onSelectChunk={handleSelectChunk}
        />

        {isEmpty || !selectedChunk ? (
          <article className="mw-pane-detail" data-testid="memory-chunk-detail-empty">
            <div className="mw-pane-scroll mw-detail-scroll">
              <MemoryEmptyPlaceholder />
            </div>
          </article>
        ) : (
          <MemoryChunkDetail chunk={selectedChunk} onSelectEntity={handleSelectEntity} />
        )}
      </div>
    </section>
  );
}
