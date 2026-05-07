/**
 * Unified memory-source list.
 *
 * One row per connected source identity, joining two RPCs:
 *
 *   - `composio.list_connections` — gives us the live OAuth identities
 *     (id + toolkit + accountEmail/workspace/username), used as the
 *     row key and to enable the per-row Sync button.
 *
 *   - `memory_tree.memory_sync_status_list` — gives us aggregated
 *     stats per toolkit (chunks synced, freshness pill, active wave
 *     progress). Stats are matched onto rows by toolkit slug, so two
 *     Gmail accounts will share the same chunk-count number until
 *     the Rust side splits stats by account_email.
 *
 * Toolkits that have chunks in the memory tree but no live Composio
 * connection (rare — usually a legacy or revoked auth) still render
 * as anonymous rows so the user sees the data exists.
 *
 * Replaces both the old `MemorySyncConnections` card and the standalone
 * "Connected sources" panel with one section, one Sync button, one
 * stats block per identity. Sync only appears when:
 *   1. the connection is currently ACTIVE/CONNECTED, AND
 *   2. the toolkit is in the syncable allow-list (today: gmail).
 */
import { useCallback, useEffect, useMemo, useState } from 'react';

import { listConnections, syncConnection } from '../../lib/composio/composioApi';
import type { ComposioConnection } from '../../lib/composio/types';
import {
  type FreshnessLabel,
  type MemorySyncStatus,
  memorySyncStatusList,
} from '../../services/memorySyncService';
import type { ToastNotification } from '../../types/intelligence';

interface MemorySourcesProps {
  /** Toolkits whose Composio sync writes into the memory tree. */
  syncableToolkits: ReadonlySet<string>;
  /** Refetch cadence for the stats poll. */
  pollIntervalMs?: number;
  /** Toast hook (success/failure). */
  onToast?: (toast: Omit<ToastNotification, 'id'>) => void;
}

const TOOLKIT_LABEL: Record<string, string> = {
  gmail: 'Gmail',
  slack: 'Slack',
  notion: 'Notion',
  github: 'GitHub',
  discord: 'Discord',
  telegram: 'Telegram',
  whatsapp: 'WhatsApp',
  meeting_notes: 'Meeting notes',
  drive_docs: 'Drive docs',
  chat: 'Chat',
  email: 'Email',
  document: 'Document',
};

const FRESHNESS_LABEL: Record<FreshnessLabel, string> = {
  active: 'Active',
  recent: 'Recent',
  idle: 'Idle',
};

function freshnessBadge(label: FreshnessLabel): string {
  switch (label) {
    case 'active':
      return 'bg-primary-100 text-primary-700';
    case 'recent':
      return 'bg-sage-100 text-sage-700';
    case 'idle':
      return 'bg-stone-100 text-stone-700';
  }
}

function relativeTimestamp(epochMs: number | null): string | null {
  if (epochMs === null) return null;
  const delta = Date.now() - epochMs;
  if (delta < 1000) return 'just now';
  const seconds = Math.floor(delta / 1000);
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

/** Identity field — first of accountEmail/workspace/username present. */
function identityFor(conn: ComposioConnection): string | null {
  return conn.accountEmail ?? conn.workspace ?? conn.username ?? null;
}

/** A row to render: connection identity (when known) plus its toolkit stats. */
interface SourceRow {
  /** Stable React key. */
  key: string;
  toolkit: string;
  /** Display title — `"Gmail · stevent95@gmail.com"` or just `"Gmail"`. */
  title: string;
  /** Composio connection backing the row, when there is one. */
  connection: ComposioConnection | null;
  /** Aggregated stats for this toolkit, when chunks exist. */
  status: MemorySyncStatus | null;
}

function buildRows(
  connections: ComposioConnection[],
  statuses: MemorySyncStatus[],
  syncableToolkits: ReadonlySet<string>
): SourceRow[] {
  // Hide rows the user can't act on: only render identities that are
  // (1) currently connected via Composio AND (2) whose toolkit has a
  // memory-tree sync implementation. Orphan toolkits with chunks but
  // no live auth, and connected toolkits without a sync provider, are
  // both filtered out — neither offers a working Sync button so they
  // were just clutter at the top of the Memory tab.
  const statusByToolkit = new Map<string, MemorySyncStatus>();
  for (const s of statuses) statusByToolkit.set(s.provider, s);

  const rows: SourceRow[] = [];
  for (const conn of connections) {
    if (!syncableToolkits.has(conn.toolkit)) continue;
    const label = TOOLKIT_LABEL[conn.toolkit] ?? conn.toolkit;
    const identity = identityFor(conn);
    const title = identity ? `${label} · ${identity}` : label;
    rows.push({
      key: `conn:${conn.id}`,
      toolkit: conn.toolkit,
      title,
      connection: conn,
      status: statusByToolkit.get(conn.toolkit) ?? null,
    });
  }
  return rows;
}

export function MemorySources({
  syncableToolkits,
  pollIntervalMs = 5000,
  onToast,
}: MemorySourcesProps) {
  const [connections, setConnections] = useState<ComposioConnection[]>([]);
  const [statuses, setStatuses] = useState<MemorySyncStatus[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [syncingId, setSyncingId] = useState<string | null>(null);

  const loadAll = useCallback(async () => {
    try {
      const [conns, stats] = await Promise.all([
        listConnections()
          .then(r => r.connections)
          .catch(err => {
            // Composio may be unreachable in dev; degrade to anonymous
            // toolkit rows from sync-status alone rather than masking
            // the rest of the UI behind an error.
            console.warn('[ui-flow][memory-sources] list_connections failed', err);
            return [] as ComposioConnection[];
          }),
        memorySyncStatusList(),
      ]);
      setConnections(conns);
      setStatuses(stats);
      setLoadError(null);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.error('[ui-flow][memory-sources] load failed', message);
      setLoadError(message);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadAll();
  }, [loadAll]);

  useEffect(() => {
    if (!pollIntervalMs) return undefined;
    const id = setInterval(() => {
      void loadAll();
    }, pollIntervalMs);
    return () => clearInterval(id);
  }, [pollIntervalMs, loadAll]);

  const rows = useMemo(
    () => buildRows(connections, statuses, syncableToolkits),
    [connections, statuses, syncableToolkits]
  );

  const handleSync = useCallback(
    async (conn: ComposioConnection, title: string) => {
      setSyncingId(conn.id);
      try {
        await syncConnection(conn.id, 'manual');
        onToast?.({
          type: 'success',
          title: `Synced ${title}`,
          message: 'New raw items will be admitted into the memory tree shortly.',
        });
        // Refresh stats immediately so the freshness pill updates
        // without waiting for the next poll tick.
        void loadAll();
      } catch (err) {
        console.error('[ui-flow][memory-sources] sync failed conn=%s', conn.id, err);
        onToast?.({
          type: 'error',
          title: `Sync failed: ${title}`,
          message: err instanceof Error ? err.message : String(err),
        });
      } finally {
        setSyncingId(prev => (prev === conn.id ? null : prev));
      }
    },
    [onToast, loadAll]
  );

  if (loading) {
    return (
      <section
        className="rounded-lg border border-stone-200 bg-white p-4"
        data-testid="memory-sources">
        <h3 className="text-sm font-semibold text-stone-700">Memory sources</h3>
        <p className="mt-2 text-xs text-stone-500">Loading…</p>
      </section>
    );
  }

  if (loadError) {
    return (
      <section
        className="rounded-lg border border-stone-200 bg-white p-4"
        data-testid="memory-sources">
        <h3 className="text-sm font-semibold text-stone-700">Memory sources</h3>
        <p className="mt-2 break-words rounded-md bg-coral-50 p-2 text-xs text-coral-800">
          {loadError}
        </p>
      </section>
    );
  }

  if (rows.length === 0) {
    return (
      <section
        className="rounded-lg border border-stone-200 bg-white p-4"
        data-testid="memory-sources">
        <h3 className="text-sm font-semibold text-stone-700">Memory sources</h3>
        <p className="mt-2 text-xs text-stone-500">
          No connected sources with a memory-tree sync provider yet. Connect Gmail (or another
          supported integration) in the Chat tab to start ingesting.
        </p>
      </section>
    );
  }

  return (
    <section
      className="rounded-lg border border-stone-200 bg-white p-4"
      data-testid="memory-sources">
      <header className="mb-3 flex items-center justify-between">
        <h3 className="text-sm font-semibold text-stone-700">Memory sources</h3>
        <span className="text-xs text-stone-400">
          {rows.length} identit{rows.length === 1 ? 'y' : 'ies'}
        </span>
      </header>
      <ul className="divide-y divide-stone-100">
        {rows.map(row => (
          <SourceRowCard
            key={row.key}
            row={row}
            isSyncing={row.connection?.id != null && syncingId === row.connection.id}
            onSync={handleSync}
          />
        ))}
      </ul>
    </section>
  );
}

interface SourceRowCardProps {
  row: SourceRow;
  isSyncing: boolean;
  onSync: (conn: ComposioConnection, title: string) => void;
}

function SourceRowCard({ row, isSyncing, onSync }: SourceRowCardProps) {
  // `buildRows` already filtered down to (connected toolkit + syncable),
  // so `connection` is non-null and `isSyncable` is always true here.
  const { connection, status, title, toolkit } = row;
  if (!connection) return null;

  const lastSync = status ? relativeTimestamp(status.last_chunk_at_ms) : null;
  const lifetime = status?.chunks_synced ?? 0;
  const pending = status?.chunks_pending ?? 0;
  const batchTotal = status?.batch_total ?? 0;
  const batchProcessed = status?.batch_processed ?? 0;
  const batchPending = batchTotal - batchProcessed;
  const pct = batchTotal > 0 ? Math.round((batchProcessed / batchTotal) * 100) : 0;
  const showProgress = batchTotal > 0 && batchPending > 0;
  const isActive = connection.status === 'ACTIVE' || connection.status === 'CONNECTED';

  return (
    <li
      className="flex flex-col gap-2 py-3 sm:flex-row sm:items-start sm:justify-between"
      data-testid={`memory-source-row-${toolkit}`}>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="truncate text-sm font-medium text-stone-900">{title}</span>
          {status && (
            <span
              className={`rounded-md px-2 py-0.5 text-xs font-medium ${freshnessBadge(status.freshness)}`}
              data-testid={`memory-source-freshness-${toolkit}`}>
              {FRESHNESS_LABEL[status.freshness]}
            </span>
          )}
          {!isActive && (
            <span className="rounded-md bg-amber-100 px-2 py-0.5 text-xs font-medium text-amber-700">
              {connection.status}
            </span>
          )}
        </div>
        <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs text-stone-500">
          <span data-testid={`memory-source-chunks-${toolkit}`}>
            {lifetime.toLocaleString()} chunks
          </span>
          {lastSync && <span>Last chunk {lastSync}</span>}
          {pending > 0 && <span>{pending.toLocaleString()} pending</span>}
        </div>
        {showProgress && (
          <div className="mt-2 max-w-md" data-testid={`memory-source-progress-${toolkit}`}>
            <div className="h-1.5 w-full overflow-hidden rounded-full bg-stone-100">
              <div
                className="h-full bg-primary-500 transition-all"
                style={{ width: `${pct}%` }}
                role="progressbar"
                aria-valuenow={batchProcessed}
                aria-valuemin={0}
                aria-valuemax={batchTotal}
              />
            </div>
            <div className="mt-1 text-xs text-stone-500">
              {batchProcessed.toLocaleString()} of {batchTotal.toLocaleString()} processed
            </div>
          </div>
        )}
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <button
          type="button"
          onClick={() => onSync(connection, title)}
          disabled={isSyncing || !isActive}
          data-testid={`memory-source-sync-${toolkit}`}
          className="inline-flex items-center gap-1.5 rounded-md
                     bg-primary-500 px-3 py-1.5 text-xs font-semibold text-white
                     shadow-sm transition-colors hover:bg-primary-600
                     disabled:cursor-not-allowed disabled:opacity-50
                     focus:outline-none focus:ring-2 focus:ring-primary-200">
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
      </div>
    </li>
  );
}

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
