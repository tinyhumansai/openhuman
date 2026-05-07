/**
 * Obsidian-style graph view for the memory tree.
 *
 *   ┌───────────────────────────────────────────────────────┐
 *   │  Memory Sync Connections (Gmail, Slack, …)            │
 *   └───────────────────────────────────────────────────────┘
 *   ┌───────────────────────────────────────────────────────┐
 *   │  [ View vault in Obsidian → ]                         │  ← deep-link
 *   └───────────────────────────────────────────────────────┘
 *   ┌───────────────────────────────────────────────────────┐
 *   │              Force-directed graph (SVG)               │
 *   │      summary nodes, parent → child edges, click       │
 *   │              opens the .md file in Obsidian            │
 *   └───────────────────────────────────────────────────────┘
 *
 * The previous chunk-list / navigator UI has been replaced with the graph
 * for presentation purposes. Data flows through the existing
 * `openhuman.memory_tree_graph_export` JSON-RPC method which dumps every
 * sealed summary plus the absolute path to the on-disk content root —
 * the latter feeds the `obsidian://open?path=...` deep link.
 */
import { useEffect, useState } from 'react';

import type { ToastNotification } from '../../types/intelligence';
import {
  type GraphExportResponse,
  memoryTreeGraphExport,
} from '../../utils/tauriCommands';
import { MemoryGraph } from './MemoryGraph';
import { MemorySyncConnections } from './MemorySyncConnections';

interface MemoryWorkspaceProps {
  onToast?: (toast: Omit<ToastNotification, 'id'>) => void;
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

export function MemoryWorkspace({ onToast: _onToast }: MemoryWorkspaceProps) {
  const [graph, setGraph] = useState<GraphExportResponse | null>(null);
  const [error, setError] = useState<string | null>(null);

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

  return (
    <div className="space-y-4" data-testid="memory-workspace">
      <MemorySyncConnections pollIntervalMs={5000} />

      {graph && (
        <div className="flex justify-end">
          <button
            type="button"
            onClick={() => openVaultInObsidian(graph.content_root_abs)}
            data-testid="memory-open-in-obsidian"
            className="inline-flex items-center gap-2 rounded-lg border border-ocean-200
                       bg-white px-4 py-2 text-sm font-medium text-ocean-700 shadow-sm
                       transition-colors hover:bg-ocean-50 focus:outline-none
                       focus:ring-2 focus:ring-ocean-200"
            title={`obsidian://open?path=${graph.content_root_abs}`}>
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
