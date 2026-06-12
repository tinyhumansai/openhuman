/**
 * Obsidian-style graph view for the memory tree, plus controls to drive
 * the ingestion pipeline manually.
 *
 *   ┌───────────────────────────────────────────────────────┐
 *   │  MemoryTreeStatusPanel (chunk counts + freshness)     │
 *   └───────────────────────────────────────────────────────┘
 *   ┌───────────────────────────────────────────────────────┐
 *   │  MemorySourcesRegistry — unified source list          │
 *   │  (Composio + folder + GitHub + RSS + web · per-row    │
 *   │   Sync button, status chip, chunk count, freshness)   │
 *   └───────────────────────────────────────────────────────┘
 *   ┌───────────────────────────────────────────────────────┐
 *   │  WhatsAppMemorySection                                │
 *   └───────────────────────────────────────────────────────┘
 *   ┌───────────────────────────────────────────────────────┐
 *   │  ModeToggle · Reset Memory · Reset Tree · Build Trees │
 *   │  [ View vault in Obsidian ]  (shown when vault set)   │
 *   └───────────────────────────────────────────────────────┘
 *   ┌───────────────────────────────────────────────────────┐
 *   │           Force-directed summary graph (SVG)          │
 *   └───────────────────────────────────────────────────────┘
 *
 * `MemorySourcesRegistry` replaces the old Composio-only `MemorySources`
 * panel. It auto-seeds active Composio connections as sources and lets
 * users add folder, GitHub repo, RSS, and web-page sources via the
 * Add Source dialog.
 *
 * `Build summary trees` calls `memory_tree.flush_now` which enqueues a
 * `flush_stale` job with `max_age_secs=0` so every L0 buffer
 * force-seals immediately. The seal worker runs each through the
 * configured cloud or local LLM and the new summary nodes appear in
 * the graph after the worker drains.
 */
import { useEffect, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { ToastNotification } from '../../types/intelligence';
import {
  type GraphExportResponse,
  type GraphMode,
  memoryTreeGraphExport,
} from '../../utils/tauriCommands';
import { MemoryControls } from './MemoryControls';
import { MemoryGraph } from './MemoryGraph';
import { MemorySourcesRegistry } from './MemorySourcesRegistry';
import { MemoryTreeStatusPanel } from './MemoryTreeStatusPanel';
import { SyncAuditPanel } from './SyncAuditPanel';
import { WhatsAppMemorySection } from './WhatsAppMemorySection';

interface MemoryWorkspaceProps {
  onToast?: (toast: Omit<ToastNotification, 'id'>) => void;
}

export function MemoryWorkspace({ onToast }: MemoryWorkspaceProps) {
  const { t } = useT();
  const [graph, setGraph] = useState<GraphExportResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [mode, setMode] = useState<GraphMode>('tree');

  const [graphVersion, setGraphVersion] = useState(0);

  // (Re)load the graph whenever the mode toggle flips or tree events arrive.
  useEffect(() => {
    console.debug('[ui-flow][memory-workspace] graph load: entry mode=%s v=%d', mode, graphVersion);
    let cancelled = false;
    setError(null);
    void (async () => {
      try {
        const resp = await memoryTreeGraphExport(mode);
        if (cancelled) return;
        console.debug(
          '[ui-flow][memory-workspace] graph load: exit mode=%s n=%d edges=%d',
          mode,
          resp.nodes.length,
          resp.edges.length
        );
        setGraph(resp);
      } catch (err) {
        if (cancelled) return;
        console.error('[ui-flow][memory-workspace] graph load failed', err);
        setError(err instanceof Error ? err.message : String(err));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [mode, graphVersion]);

  useEffect(() => {
    const onTreeDone = () => {
      setTimeout(() => setGraphVersion(v => v + 1), 2000);
    };
    const onSyncDone = (e: Event) => {
      const data = (e as CustomEvent).detail as { stage?: string } | null;
      if (data?.stage === 'completed') {
        setTimeout(() => setGraphVersion(v => v + 1), 3000);
      }
    };
    window.addEventListener('openhuman:memory-tree-completed', onTreeDone);
    window.addEventListener('openhuman:memory-sync-stage', onSyncDone);
    return () => {
      window.removeEventListener('openhuman:memory-tree-completed', onTreeDone);
      window.removeEventListener('openhuman:memory-sync-stage', onSyncDone);
    };
  }, []);

  // Live refresh: re-pull the graph every 30s while this tab is mounted so it
  // reflects background tree growth (e.g. seal_document jobs draining as
  // Notion syncs) without a manual refresh. The Memory tab unmounts this
  // component when inactive, which clears the interval — so the poll only runs
  // while the tab is actually open. Ticks are skipped while the window is
  // backgrounded to avoid needless RPC churn; the next visible tick catches up.
  useEffect(() => {
    const GRAPH_POLL_MS = 30_000;
    const id = setInterval(() => {
      if (typeof document !== 'undefined' && document.hidden) return;
      console.debug('[ui-flow][memory-workspace] graph poll tick → bump version');
      setGraphVersion(v => v + 1);
    }, GRAPH_POLL_MS);
    return () => clearInterval(id);
  }, []);

  return (
    <div className="space-y-4" data-testid="memory-workspace">
      <MemoryTreeStatusPanel onToast={onToast} />
      <MemorySourcesRegistry onToast={onToast} />
      <WhatsAppMemorySection />

      <MemoryControls
        mode={mode}
        onModeChange={setMode}
        onRefresh={() => setGraphVersion(v => v + 1)}
        onToast={onToast}
        contentRootAbs={graph?.content_root_abs}
      />

      {error ? (
        <div className="rounded-lg border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-4 py-3 text-sm text-coral-800">
          {t('workspace.graphLoadFailed')}: {error}
        </div>
      ) : !graph ? (
        <div className="flex h-[640px] items-center justify-center rounded-lg border border-stone-100 dark:border-neutral-800 bg-stone-50/40 text-sm text-stone-500 dark:text-neutral-400">
          {t('workspace.loadingGraph')}
        </div>
      ) : (
        <MemoryGraph nodes={graph.nodes} edges={graph.edges} mode={mode} />
      )}

      <div className="rounded-lg border border-stone-100 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-4">
        <h3 className="mb-2 text-sm font-medium text-stone-700 dark:text-neutral-200">
          {t('sync.auditTitle', 'Sync History')}
        </h3>
        <SyncAuditPanel />
      </div>
    </div>
  );
}
