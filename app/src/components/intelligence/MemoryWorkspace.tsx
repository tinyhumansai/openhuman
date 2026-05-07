/**
 * Obsidian-style graph view for the memory tree, plus controls to drive
 * the ingestion pipeline manually.
 *
 *   ┌───────────────────────────────────────────────────────┐
 *   │  Memory Sync Connections (counts + freshness pills)   │
 *   └───────────────────────────────────────────────────────┘
 *   ┌───────────────────────────────────────────────────────┐
 *   │  Composio connections  · [Sync] per row               │
 *   └───────────────────────────────────────────────────────┘
 *   ┌───────────────────────────────────────────────────────┐
 *   │   [ View vault in Obsidian ]   [ Build summary trees ]│
 *   └───────────────────────────────────────────────────────┘
 *   ┌───────────────────────────────────────────────────────┐
 *   │           Force-directed summary graph (SVG)          │
 *   └───────────────────────────────────────────────────────┘
 *
 * `Sync` (per provider) calls `composio.sync` which downloads new raw
 * items from the toolkit (Gmail messages, Slack messages, …) and
 * writes them into the memory chunk store.
 *
 * `Build summary trees` calls `memory_tree.flush_now` which enqueues a
 * `flush_stale` job with `max_age_secs=0` so every L0 buffer
 * force-seals immediately. The seal worker runs each through the
 * configured cloud or local LLM and the new summary nodes appear in
 * the graph after the worker drains.
 */
import { useCallback, useEffect, useState } from 'react';

import type { ToastNotification } from '../../types/intelligence';
import { openUrl } from '../../utils/openUrl';
import {
  type GraphExportResponse,
  type GraphMode,
  memoryTreeFlushNow,
  memoryTreeGraphExport,
  memoryTreeResetTree,
  memoryTreeWipeAll,
} from '../../utils/tauriCommands';
import { MemoryGraph } from './MemoryGraph';
import { MemorySources } from './MemorySources';
import { WhatsAppMemorySection } from './WhatsAppMemorySection';

interface MemoryWorkspaceProps {
  onToast?: (toast: Omit<ToastNotification, 'id'>) => void;
}

/**
 * Toolkits that have a memory-tree-ingesting sync implementation on the
 * Rust side. Only these get a Sync button — clicking it on a toolkit
 * that lacks an ingest path would just churn the worker without
 * adding chunks to the memory tree.
 *
 * Source of truth: providers under
 * `src/openhuman/composio/providers/<toolkit>/` that call
 * `ingest_page_into_memory_tree`. Today that's gmail. Add a slug here
 * when a new provider lands a memory-tree ingest path.
 */
const SYNCABLE_TOOLKITS: ReadonlySet<string> = new Set(['gmail']);

/**
 * Trigger the `obsidian://open?path=<abs>` deep link via the OS shell.
 *
 * We deliberately route through `openUrl` (which delegates to
 * `tauri-plugin-opener`) rather than setting `window.location.href`.
 * The webview-host intent handler intercepts in-app navigations and
 * does NOT punt custom schemes to the OS, so a direct
 * `window.location.href = "obsidian://…"` either no-ops or navigates
 * the React app away from the Memory tab. The opener plugin hands the
 * URL straight to the system handler so Obsidian launches as a
 * separate process.
 */
async function openVaultInObsidian(contentRootAbs: string): Promise<void> {
  const url = `obsidian://open?path=${encodeURIComponent(contentRootAbs)}`;
  console.debug('[ui-flow][memory-workspace] open vault in Obsidian url=%s', url);
  try {
    await openUrl(url);
  } catch (err) {
    console.error('[ui-flow][memory-workspace] openUrl failed', err);
  }
}

export function MemoryWorkspace({ onToast }: MemoryWorkspaceProps) {
  const [graph, setGraph] = useState<GraphExportResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [building, setBuilding] = useState(false);
  const [wiping, setWiping] = useState(false);
  const [resetting, setResetting] = useState(false);
  const [mode, setMode] = useState<GraphMode>('tree');

  // (Re)load the graph whenever the mode toggle flips. The Memory
  // sources panel manages its own polling.
  useEffect(() => {
    console.debug('[ui-flow][memory-workspace] graph load: entry mode=%s', mode);
    let cancelled = false;
    setError(null);
    setGraph(null);
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
  }, [mode]);

  const handleWipe = useCallback(async () => {
    // Two-step confirm so accidental clicks can't nuke a workspace.
    const ok = window.confirm(
      'This deletes every chunk, summary, and raw markdown file in this workspace. ' +
        'Re-syncing afterwards will re-ingest from upstream. Continue?'
    );
    if (!ok) return;
    setWiping(true);
    try {
      const resp = await memoryTreeWipeAll();
      onToast?.({
        type: 'success',
        title: 'Memory wiped',
        message:
          `Removed ${resp.rows_deleted.toLocaleString()} row(s) and ` +
          `${resp.dirs_removed.length} folder(s); cleared ` +
          `${resp.sync_state_cleared.toLocaleString()} sync-state cursor(s). ` +
          `Click Sync on a connected source to repopulate.`,
      });
      // Re-fetch the (now empty) graph immediately so the canvas
      // reflects the wipe instead of staying frozen on stale data.
      try {
        const next = await memoryTreeGraphExport(mode);
        setGraph(next);
      } catch (err) {
        console.warn('[ui-flow][memory-workspace] post-wipe graph refresh failed', err);
      }
    } catch (err) {
      console.error('[ui-flow][memory-workspace] wipe_all failed', err);
      onToast?.({
        type: 'error',
        title: 'Reset failed',
        message: err instanceof Error ? err.message : String(err),
      });
    } finally {
      setWiping(false);
    }
  }, [onToast, mode]);

  const handleResetTree = useCallback(async () => {
    const ok = window.confirm(
      'This deletes every summary, buffer, and tree job — but keeps chunks ' +
        'and raw markdown intact. Every chunk gets re-queued through extraction ' +
        'and the tree rebuilds from scratch on the *current* summariser. ' +
        'No upstream re-fetch. Continue?'
    );
    if (!ok) return;
    setResetting(true);
    try {
      const resp = await memoryTreeResetTree();
      onToast?.({
        type: 'success',
        title: 'Memory tree rebuilding',
        message:
          `Cleared ${resp.tree_rows_deleted.toLocaleString()} tree row(s); ` +
          `requeued ${resp.chunks_requeued.toLocaleString()} chunk(s) ` +
          `(${resp.jobs_enqueued.toLocaleString()} extract jobs). ` +
          `The graph will fill back in as the worker drains.`,
      });
      // Stagger the graph re-fetch a bit longer than build_trees does —
      // reset_tree starts from extract jobs (slower than seal-only).
      setTimeout(() => {
        void (async () => {
          try {
            const next = await memoryTreeGraphExport(mode);
            setGraph(next);
          } catch (err) {
            console.warn('[ui-flow][memory-workspace] post-reset graph refresh failed', err);
          }
        })();
      }, 8000);
    } catch (err) {
      console.error('[ui-flow][memory-workspace] reset_tree failed', err);
      onToast?.({
        type: 'error',
        title: 'Could not reset memory tree',
        message: err instanceof Error ? err.message : String(err),
      });
    } finally {
      setResetting(false);
    }
  }, [onToast, mode]);

  const handleBuildTrees = useCallback(async () => {
    setBuilding(true);
    try {
      const resp = await memoryTreeFlushNow();
      onToast?.({
        type: resp.enqueued ? 'success' : 'info',
        title: resp.enqueued
          ? `Building summary trees · ${resp.stale_buffers} buffer(s)`
          : 'Build already in progress',
        message: resp.enqueued
          ? 'Force-sealing every L0 buffer through the configured AI summariser. The graph will refresh once the worker drains.'
          : 'A flush job for today is already queued — no new work needed.',
      });
      // Re-fetch the graph after a short delay so newly-sealed
      // summaries appear in the view. The seal cascade runs async on
      // the worker pool; 4s is enough for the typical case without
      // making the UI feel stuck.
      setTimeout(() => {
        void (async () => {
          try {
            const next = await memoryTreeGraphExport(mode);
            setGraph(next);
          } catch (err) {
            console.warn('[ui-flow][memory-workspace] post-build graph refresh failed', err);
          }
        })();
      }, 4000);
    } catch (err) {
      console.error('[ui-flow][memory-workspace] flush_now failed', err);
      onToast?.({
        type: 'error',
        title: 'Could not build summary trees',
        message: err instanceof Error ? err.message : String(err),
      });
    } finally {
      setBuilding(false);
    }
  }, [onToast, mode]);

  return (
    <div className="space-y-4" data-testid="memory-workspace">
      <MemorySources syncableToolkits={SYNCABLE_TOOLKITS} pollIntervalMs={5000} onToast={onToast} />
      <WhatsAppMemorySection />

      <div
        className="flex flex-wrap items-center justify-between gap-3"
        data-testid="memory-actions">
        <ModeToggle mode={mode} onChange={setMode} />
        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={handleWipe}
            disabled={wiping || building}
            data-testid="memory-wipe-all"
            className="inline-flex items-center gap-2 rounded-lg
                       border border-coral-200 bg-white px-4 py-2 text-sm font-semibold
                       text-coral-700 shadow-sm transition-colors hover:bg-coral-50
                       disabled:cursor-not-allowed disabled:opacity-50
                       focus:outline-none focus:ring-2 focus:ring-coral-200"
            title="Delete every chunk, summary, and raw file in this workspace">
            {wiping ? (
              <>
                <Spinner /> Resetting…
              </>
            ) : (
              <>
                <TrashIcon /> Reset memory
              </>
            )}
          </button>
          <button
            type="button"
            onClick={handleResetTree}
            disabled={resetting || wiping || building}
            data-testid="memory-reset-tree"
            className="inline-flex items-center gap-2 rounded-lg
                       border border-amber-300 bg-white px-4 py-2 text-sm font-semibold
                       text-amber-800 shadow-sm transition-colors hover:bg-amber-50
                       disabled:cursor-not-allowed disabled:opacity-50
                       focus:outline-none focus:ring-2 focus:ring-amber-200"
            title="Wipe summaries + buffers and re-summarise existing chunks (no upstream re-fetch)">
            {resetting ? (
              <>
                <Spinner /> Rebuilding…
              </>
            ) : (
              <>
                <RefreshIcon /> Reset memory tree
              </>
            )}
          </button>
          <button
            type="button"
            onClick={handleBuildTrees}
            disabled={building}
            data-testid="memory-build-trees"
            className="inline-flex items-center gap-2 rounded-lg
                       bg-primary-500 px-4 py-2 text-sm font-semibold text-white
                       shadow-sm transition-colors hover:bg-primary-600
                       disabled:cursor-not-allowed disabled:opacity-50
                       focus:outline-none focus:ring-2 focus:ring-primary-200">
            {building ? (
              <>
                <Spinner /> Building…
              </>
            ) : (
              <>
                <BrainIcon /> Build summary trees
              </>
            )}
          </button>
          {graph && (
            <button
              type="button"
              onClick={() => void openVaultInObsidian(graph.content_root_abs)}
              data-testid="memory-open-in-obsidian"
              className="inline-flex items-center gap-2 rounded-lg
                         bg-violet-500 px-4 py-2 text-sm font-semibold text-white
                         shadow-sm transition-colors hover:bg-violet-600
                         focus:outline-none focus:ring-2 focus:ring-violet-300"
              title={`obsidian://open?path=${graph.content_root_abs}`}>
              <ExternalLinkIcon />
              View vault in Obsidian
            </button>
          )}
        </div>
      </div>

      {error ? (
        <div className="rounded-lg border border-coral-200 bg-coral-50 px-4 py-3 text-sm text-coral-800">
          Failed to load memory graph: {error}
        </div>
      ) : !graph ? (
        <div className="flex h-[640px] items-center justify-center rounded-lg border border-stone-100 bg-stone-50/40 text-sm text-stone-500">
          Loading graph…
        </div>
      ) : (
        <MemoryGraph
          nodes={graph.nodes}
          edges={graph.edges}
          mode={mode}
          contentRootAbs={graph.content_root_abs}
        />
      )}
    </div>
  );
}

interface ModeToggleProps {
  mode: GraphMode;
  onChange: (next: GraphMode) => void;
}

function ModeToggle({ mode, onChange }: ModeToggleProps) {
  const baseBtn =
    'px-3 py-1.5 text-xs font-medium rounded-md transition-colors focus:outline-none focus:ring-2 focus:ring-primary-200';
  const active = 'bg-primary-500 text-white shadow-sm';
  const idle = 'bg-white text-stone-600 hover:bg-stone-50';
  return (
    <div
      className="inline-flex items-center gap-1 rounded-lg border border-stone-200 bg-stone-50 p-1"
      role="tablist"
      aria-label="Graph view mode"
      data-testid="memory-graph-mode-toggle">
      <button
        type="button"
        onClick={() => onChange('tree')}
        className={`${baseBtn} ${mode === 'tree' ? active : idle}`}
        role="tab"
        aria-selected={mode === 'tree'}
        data-testid="memory-graph-mode-tree">
        Trees
      </button>
      <button
        type="button"
        onClick={() => onChange('contacts')}
        className={`${baseBtn} ${mode === 'contacts' ? active : idle}`}
        role="tab"
        aria-selected={mode === 'contacts'}
        data-testid="memory-graph-mode-contacts">
        Contacts
      </button>
    </div>
  );
}

// ── Tiny inline icons (no extra dep) ────────────────────────────────────

function RefreshIcon() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true">
      <path d="M21 12a9 9 0 11-3-6.7" />
      <path d="M21 4v5h-5" />
      <path d="M3 12a9 9 0 003 6.7" />
      <path d="M3 20v-5h5" />
    </svg>
  );
}

function TrashIcon() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true">
      <path d="M3 6h18" />
      <path d="M8 6V4a2 2 0 012-2h4a2 2 0 012 2v2" />
      <path d="M19 6l-1 14a2 2 0 01-2 2H8a2 2 0 01-2-2L5 6" />
      <path d="M10 11v6" />
      <path d="M14 11v6" />
    </svg>
  );
}

function BrainIcon() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true">
      <path d="M9 4.5a2.5 2.5 0 015 0v15a2.5 2.5 0 01-5 0" />
      <path d="M9 4.5A2.5 2.5 0 116.5 7M9 19.5A2.5 2.5 0 116.5 17" />
      <path d="M14 4.5A2.5 2.5 0 1117.5 7M14 19.5A2.5 2.5 0 1017.5 17" />
    </svg>
  );
}

function ExternalLinkIcon() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true">
      <path d="M14 3h7v7" />
      <path d="M10 14L21 3" />
      <path d="M21 14v7H3V3h7" />
    </svg>
  );
}

function Spinner() {
  return (
    <svg
      className="animate-spin"
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      aria-hidden="true">
      <circle cx="12" cy="12" r="9" opacity="0.25" />
      <path d="M21 12a9 9 0 00-9-9" />
    </svg>
  );
}
