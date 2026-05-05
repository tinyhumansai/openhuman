/**
 * Two-pane memory_tree browser with overlay detail.
 *
 *   ┌─────────────┬──────────────────────────────────────┐
 *   │  NAVIGATOR  │  RESULT LIST                         │  ← default 2-pane
 *   │  240px      │  flex                                │
 *   └─────────────┴──────────────────────────────────────┘
 *
 *   ┌──────────────────────────────────────────────────[✕]┐
 *   │  CHUNK DETAIL (full card width)                     │  ← when chunk selected
 *   │  Subject · sender · entities · body · score         │
 *   └─────────────────────────────────────────────────────┘
 *
 * ResultList finally gets ~970px to breathe (multi-line rows with
 * sender, time, entity chips, preview). When a chunk is selected the
 * detail layer absolute-positions over the 2-pane base so list scroll
 * state is preserved on close. Esc + close button + (later) backdrop
 * click all dismiss.
 *
 * Talks to the real `openhuman.memory_tree_*` JSON-RPC surface via the
 * `utils/tauriCommands/memoryTree` wrappers.
 */
import { useCallback, useEffect, useMemo, useState } from 'react';

import type { ToastNotification } from '../../types/intelligence';
import {
  type Chunk,
  type ChunkFilter,
  type EntityRef,
  memoryTreeChunksForEntity,
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
  onToast?: (toast: Omit<ToastNotification, 'id'>) => void;
}

export function MemoryWorkspace({ onToast: _onToast }: MemoryWorkspaceProps) {
  const [allChunks, setAllChunks] = useState<Chunk[]>([]);
  const [sources, setSources] = useState<Source[]>([]);
  const [topPeople, setTopPeople] = useState<EntityRef[]>([]);
  const [topTopics, setTopTopics] = useState<EntityRef[]>([]);
  const [selectedChunkId, setSelectedChunkId] = useState<string | null>(null);
  const [selection, setSelection] = useState<NavigatorSelection>({ sourceIds: [], entityIds: [] });
  const [searchQuery, setSearchQuery] = useState('');

  // Initial data load.
  useEffect(() => {
    console.debug('[ui-flow][memory-workspace] initial load (2-pane + overlay)');
    let cancelled = false;
    const run = async () => {
      try {
        const [chunkResult, srcs, people, anyEntities] = await Promise.all([
          memoryTreeListChunks({ limit: 500 }),
          memoryTreeListSources(),
          memoryTreeTopEntities('person', 12),
          memoryTreeTopEntities(undefined, 40),
        ]);
        if (cancelled) return;
        const topicKinds = new Set(['technology', 'product', 'event']);
        const topics = anyEntities.filter(e => topicKinds.has(e.kind)).slice(0, 12);
        setAllChunks(chunkResult.chunks);
        setSources(srcs);
        setTopPeople(people);
        setTopTopics(topics);
      } catch (err) {
        if (cancelled) return;
        // Initial-load failure leaves the panes empty rather than
        // half-populated with stale state. The console line lets us
        // diagnose without blocking the user behind a modal — they can
        // still navigate the tab and retry by reloading.
        console.error('[ui-flow][memory-workspace] initial load failed', err);
      }
    };
    void run();
    return () => {
      cancelled = true;
    };
  }, []);

  // Resolve entity selection → set of chunk ids via the dedicated
  // `memory_tree_chunks_for_entity` RPC. The chunks' `tags` column only
  // stores high-level category tags (`["gmail", "ingested"]`), NOT
  // per-chunk entity refs — those live in `mem_tree_entity_index`.
  // Calling the inverse-index RPC gives us the real chunk ids that
  // mention each selected entity. Union across the selection (chunk
  // mentions ANY of the selected entities is enough — same semantics
  // as a multi-select OR filter in Mail.app's people sidebar).
  const [entityChunkIds, setEntityChunkIds] = useState<Set<string> | null>(null);
  useEffect(() => {
    console.debug(
      '[ui-flow][memory-workspace] entity-effect fire entityIds=%o',
      selection.entityIds
    );
    let cancelled = false;
    const run = async () => {
      if (selection.entityIds.length === 0) {
        setEntityChunkIds(null);
        return;
      }
      try {
        const results = await Promise.all(
          selection.entityIds.map(id => memoryTreeChunksForEntity(id))
        );
        if (cancelled) {
          console.debug('[ui-flow][memory-workspace] entity-effect cancelled before commit');
          return;
        }
        const union = new Set<string>();
        for (const ids of results) for (const id of ids) union.add(id);
        console.debug(
          '[ui-flow][memory-workspace] entity-effect commit set_size=%d sample=%o',
          union.size,
          [...union].slice(0, 3)
        );
        setEntityChunkIds(union);
      } catch (err) {
        if (cancelled) return;
        // If the inverse-index lookup rejects, do NOT keep filtering by
        // the previously-resolved set — that would leave the result list
        // showing chunks tied to the old selection while the user thinks
        // they've moved on. Reset to "no entity filter" so they at least
        // see the unfiltered timeline; the navigator selection is left
        // alone so they can retry by reselecting.
        console.error('[ui-flow][memory-workspace] entity-effect lookup failed', err);
        setEntityChunkIds(null);
      }
    };
    void run();
    return () => {
      cancelled = true;
    };
  }, [selection.entityIds]);

  // Apply navigator selection + search.
  const filteredChunks = useMemo<Chunk[]>(() => {
    const filter: ChunkFilter = {
      source_ids: selection.sourceIds.length > 0 ? selection.sourceIds : undefined,
      query: searchQuery.trim() || undefined,
    };
    const out = allChunks.filter(c => {
      if (filter.source_ids && !filter.source_ids.includes(c.source_id)) return false;
      if (entityChunkIds && !entityChunkIds.has(c.id)) return false;
      if (filter.query) {
        const needle = filter.query.toLowerCase();
        const hay = `${c.content_preview ?? ''} ${c.tags.join(' ')}`.toLowerCase();
        if (!hay.includes(needle)) return false;
      }
      return true;
    });
    console.debug(
      '[ui-flow][memory-workspace] filteredChunks recompute all=%d entitySet=%s out=%d',
      allChunks.length,
      entityChunkIds ? `Set(${entityChunkIds.size})` : 'null',
      out.length
    );
    return out;
  }, [allChunks, selection.sourceIds, entityChunkIds, searchQuery]);

  const selectedChunk = useMemo(
    () => allChunks.find(c => c.id === selectedChunkId) ?? null,
    [allChunks, selectedChunkId]
  );

  const handleSelectChunk = useCallback((id: string) => {
    console.debug('[ui-flow][memory-workspace] open chunk overlay', id);
    setSelectedChunkId(id);
  }, []);

  const handleCloseDetail = useCallback(() => {
    setSelectedChunkId(null);
  }, []);

  const handleSelectionChange = useCallback((next: NavigatorSelection) => {
    setSelection(next);
  }, []);

  const handleSearchChange = useCallback((q: string) => {
    setSearchQuery(q);
  }, []);

  const handleSelectEntity = useCallback((entity: EntityRef) => {
    console.debug('[ui-flow][memory-workspace] entity click → activate lens', entity.entity_id);
    setSelection(prev => {
      if (prev.entityIds.includes(entity.entity_id)) return prev;
      return { ...prev, entityIds: [...prev.entityIds, entity.entity_id] };
    });
    // Closing detail surfaces the filtered list immediately.
    setSelectedChunkId(null);
  }, []);

  // Esc key dismisses the detail overlay.
  useEffect(() => {
    if (!selectedChunkId) return;
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        handleCloseDetail();
      }
    };
    window.addEventListener('keydown', handleKey);
    return () => window.removeEventListener('keydown', handleKey);
  }, [selectedChunkId, handleCloseDetail]);

  const isEmpty = allChunks.length === 0;

  if (isEmpty) {
    return (
      <div className="flex min-h-[40vh] items-center justify-center">
        <MemoryEmptyPlaceholder />
      </div>
    );
  }

  return (
    <section
      className="memory-workspace-root relative flex"
      style={{ height: 'calc(100vh - 16rem)' }}
      data-testid="memory-workspace">
      {/* 2-pane base */}
      <aside
        className="w-60 shrink-0 overflow-y-auto border-r border-stone-100 bg-stone-50/60"
        aria-label="Memory navigator">
        <MemoryNavigator
          chunks={allChunks}
          sources={sources}
          topPeople={topPeople}
          topTopics={topTopics}
          selection={selection}
          onSelectionChange={handleSelectionChange}
          searchQuery={searchQuery}
          onSearchChange={handleSearchChange}
        />
      </aside>

      <main className="flex-1 overflow-y-auto bg-white" aria-label="Result list">
        <MemoryResultList
          chunks={filteredChunks}
          selectedChunkId={selectedChunkId}
          onSelectChunk={handleSelectChunk}
        />
      </main>

      {/* Detail overlay — fills the entire workspace card */}
      {selectedChunk && (
        <div
          className="absolute inset-0 z-10 flex flex-col bg-canvas-50/95 backdrop-blur-sm
                     duration-150 motion-safe:animate-fade-in"
          role="dialog"
          aria-modal="true"
          aria-label="Chunk detail">
          <header
            className="sticky top-0 z-10 flex items-center justify-between gap-4
                       border-b border-stone-100 bg-white/90 px-6 py-3 backdrop-blur">
            <div className="min-w-0 flex-1">
              <p className="truncate font-mono text-xs text-stone-500">
                <span className="text-stone-400">{selectedChunk.source_kind}</span>
                {' · '}
                {selectedChunk.source_id}
              </p>
            </div>
            <button
              onClick={handleCloseDetail}
              aria-label="Close detail (Esc)"
              title="Close (Esc)"
              className="rounded-lg p-1.5 text-stone-500 transition-colors
                         hover:bg-stone-100 hover:text-stone-900
                         focus:outline-none focus:ring-2 focus:ring-ocean-200">
              <svg
                width="18"
                height="18"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
                aria-hidden="true">
                <path d="M18 6L6 18" />
                <path d="M6 6l12 12" />
              </svg>
            </button>
          </header>

          <div className="flex-1 overflow-y-auto">
            <div className="mx-auto max-w-4xl px-8 py-6">
              <MemoryChunkDetail chunk={selectedChunk} onSelectEntity={handleSelectEntity} />
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
