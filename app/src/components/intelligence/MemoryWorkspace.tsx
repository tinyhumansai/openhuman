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

import { listConnections, syncConnection } from '../../lib/composio/composioApi';
import type { ComposioConnection } from '../../lib/composio/types';
import type { ToastNotification } from '../../types/intelligence';
import {
  type GraphExportResponse,
  memoryTreeFlushNow,
  memoryTreeGraphExport,
} from '../../utils/tauriCommands';
import { MemoryGraph } from './MemoryGraph';
import { MemorySyncConnections } from './MemorySyncConnections';

interface MemoryWorkspaceProps {
  onToast?: (toast: Omit<ToastNotification, 'id'>) => void;
}

const TOOLKIT_LABEL: Record<string, string> = {
  gmail: 'Gmail',
  slack: 'Slack',
  notion: 'Notion',
  github: 'GitHub',
};

function labelFor(connection: ComposioConnection): string {
  const base = TOOLKIT_LABEL[connection.toolkit] ?? connection.toolkit;
  const identity =
    connection.accountEmail ?? connection.workspace ?? connection.username ?? null;
  return identity ? `${base} · ${identity}` : base;
}

/**
 * Trigger the `obsidian://open?path=<abs>` deep link. Obsidian opens
 * arbitrary absolute paths without requiring the vault to be registered
 * up-front, so the same URL works on a fresh machine as long as the user
 * has Obsidian installed.
 */
function openVaultInObsidian(contentRootAbs: string) {
  const url = `obsidian://open?path=${encodeURIComponent(contentRootAbs)}`;
  console.debug('[ui-flow][memory-workspace] open vault in Obsidian url=%s', url);
  window.location.href = url;
}

export function MemoryWorkspace({ onToast }: MemoryWorkspaceProps) {
  const [graph, setGraph] = useState<GraphExportResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [connections, setConnections] = useState<ComposioConnection[]>([]);
  const [syncingId, setSyncingId] = useState<string | null>(null);
  const [building, setBuilding] = useState(false);

  // Load both the graph and the connections list once on mount. They
  // power separate UI rows but both fail-soft — if connections RPC
  // fails we still render the graph, and vice versa.
  useEffect(() => {
    console.debug('[ui-flow][memory-workspace] graph load: entry');
    let cancelled = false;
    void (async () => {
      try {
        const resp = await memoryTreeGraphExport();
        if (cancelled) return;
        console.debug(
          '[ui-flow][memory-workspace] graph load: exit n=%d root=%s',
          resp.nodes.length,
          resp.content_root_abs
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
  }, []);

  const refreshConnections = useCallback(async () => {
    try {
      const { connections: list } = await listConnections();
      setConnections(list);
    } catch (err) {
      console.error('[ui-flow][memory-workspace] list_connections failed', err);
    }
  }, []);

  useEffect(() => {
    void refreshConnections();
  }, [refreshConnections]);

  const handleSync = useCallback(
    async (connection: ComposioConnection) => {
      setSyncingId(connection.id);
      try {
        await syncConnection(connection.id, 'manual');
        onToast?.({
          type: 'success',
          title: `Synced ${labelFor(connection)}`,
          message: 'New raw items will be admitted into the memory tree shortly.',
        });
      } catch (err) {
        console.error('[ui-flow][memory-workspace] sync failed conn=%s', connection.id, err);
        onToast?.({
          type: 'error',
          title: `Sync failed: ${labelFor(connection)}`,
          message: err instanceof Error ? err.message : String(err),
        });
      } finally {
        setSyncingId(prev => (prev === connection.id ? null : prev));
      }
    },
    [onToast]
  );

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
            const next = await memoryTreeGraphExport();
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
  }, [onToast]);

  return (
    <div className="space-y-4" data-testid="memory-workspace">
      <MemorySyncConnections pollIntervalMs={5000} />

      {connections.length > 0 && (
        <section
          className="rounded-lg border border-stone-100 bg-white p-4"
          data-testid="memory-source-connections">
          <header className="mb-3 flex items-center justify-between">
            <h3 className="text-sm font-medium text-stone-700">Connected sources</h3>
            <span className="text-xs text-stone-400">click sync to download new items</span>
          </header>
          <ul className="divide-y divide-stone-100">
            {connections.map(conn => {
              const isSyncing = syncingId === conn.id;
              const isActive =
                conn.status === 'ACTIVE' || conn.status === 'CONNECTED';
              return (
                <li
                  key={conn.id}
                  className="flex items-center justify-between gap-3 py-2"
                  data-testid={`memory-source-row-${conn.toolkit}`}>
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium text-stone-800">
                      {labelFor(conn)}
                    </p>
                    <p className="truncate font-mono text-xs text-stone-400">
                      {conn.status}
                      {conn.createdAt ? ` · added ${conn.createdAt.slice(0, 10)}` : ''}
                    </p>
                  </div>
                  <button
                    type="button"
                    onClick={() => handleSync(conn)}
                    disabled={isSyncing || !isActive}
                    data-testid={`memory-source-sync-${conn.toolkit}`}
                    className="inline-flex items-center gap-1.5 rounded-md border border-stone-200
                               bg-white px-3 py-1.5 text-xs font-medium text-stone-700 shadow-sm
                               transition-colors hover:bg-stone-50 disabled:cursor-not-allowed
                               disabled:opacity-50 focus:outline-none focus:ring-2
                               focus:ring-ocean-200">
                    {isSyncing ? (
                      <>
                        <Spinner /> Syncing…
                      </>
                    ) : (
                      <>
                        <SyncIcon /> Sync
                      </>
                    )}
                  </button>
                </li>
              );
            })}
          </ul>
        </section>
      )}

      {graph && (
        <div
          className="flex flex-wrap items-center justify-end gap-2"
          data-testid="memory-actions">
          <button
            type="button"
            onClick={handleBuildTrees}
            disabled={building}
            data-testid="memory-build-trees"
            className="inline-flex items-center gap-2 rounded-lg border border-ocean-200
                       bg-ocean-50 px-4 py-2 text-sm font-medium text-ocean-800 shadow-sm
                       transition-colors hover:bg-ocean-100 disabled:cursor-not-allowed
                       disabled:opacity-50 focus:outline-none focus:ring-2
                       focus:ring-ocean-200">
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
          <button
            type="button"
            onClick={() => openVaultInObsidian(graph.content_root_abs)}
            data-testid="memory-open-in-obsidian"
            className="inline-flex items-center gap-2 rounded-lg border border-stone-200
                       bg-white px-4 py-2 text-sm font-medium text-stone-700 shadow-sm
                       transition-colors hover:bg-stone-50 focus:outline-none
                       focus:ring-2 focus:ring-ocean-200"
            title={`obsidian://open?path=${graph.content_root_abs}`}>
            <ExternalLinkIcon />
            View vault in Obsidian
          </button>
        </div>
      )}

      {error ? (
        <div className="rounded-lg border border-coral-200 bg-coral-50 px-4 py-3 text-sm text-coral-800">
          Failed to load memory graph: {error}
        </div>
      ) : !graph ? (
        <div className="flex h-[640px] items-center justify-center rounded-lg border border-stone-100 bg-stone-50/40 text-sm text-stone-500">
          Loading graph…
        </div>
      ) : (
        <MemoryGraph nodes={graph.nodes} contentRootAbs={graph.content_root_abs} />
      )}
    </div>
  );
}

// ── Tiny inline icons (no extra dep) ────────────────────────────────────

function SyncIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true">
      <path d="M21 12a9 9 0 11-3-6.7" />
      <path d="M21 4v5h-5" />
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
