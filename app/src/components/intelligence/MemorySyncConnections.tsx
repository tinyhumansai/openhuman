/**
 * Memory sync card list (#1136 — simplified rewrite).
 *
 * Renders one card per `source_kind` (data-source type) that has chunks
 * in the memory tree. Counts come straight from a SQL aggregate over
 * `mem_tree_chunks` so the snapshot is always exact at the moment of
 * the poll. No phases, no settings, no per-connection state — chunks
 * exist or they don't.
 */
import { useCallback, useEffect, useState } from 'react';

import {
  type FreshnessLabel,
  type MemorySyncStatus,
  memorySyncStatusList,
} from '../../services/memorySyncService';

interface MemorySyncConnectionsProps {
  /** Optional pollIntervalMs — when set, the list refetches periodically. */
  pollIntervalMs?: number;
}

const FRESHNESS_LABEL: Record<FreshnessLabel, string> = {
  active: 'Active',
  recent: 'Recent',
  idle: 'Idle',
};

const PROVIDER_LABEL: Record<string, string> = {
  slack: 'Slack',
  discord: 'Discord',
  telegram: 'Telegram',
  whatsapp: 'WhatsApp',
  gmail: 'Gmail',
  other_email: 'Email',
  notion: 'Notion',
  meeting_notes: 'Meeting notes',
  drive_docs: 'Drive docs',
  // category fallbacks (for chunks without a `:` prefix in source_id)
  chat: 'Chat',
  email: 'Email',
  document: 'Document',
};

function freshnessBadgeClass(label: FreshnessLabel): string {
  switch (label) {
    case 'active':
      return 'bg-ocean-100 text-ocean-700';
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

interface SourceCardProps {
  status: MemorySyncStatus;
}

function SourceCard({ status }: SourceCardProps) {
  const label = PROVIDER_LABEL[status.provider] ?? status.provider;
  const lastSync = relativeTimestamp(status.last_chunk_at_ms);
  const lifetime = status.chunks_synced;
  const pending = status.chunks_pending;
  // Progress reflects the *active sync wave* (chunks within the most
  // recent ingest cluster), not lifetime, so the bar tracks "how much
  // of this sync's ingest has been processed". Hidden once the wave
  // is fully drained.
  const batchTotal = status.batch_total;
  const batchProcessed = status.batch_processed;
  const batchPending = batchTotal - batchProcessed;
  const pct = batchTotal > 0 ? Math.round((batchProcessed / batchTotal) * 100) : 0;
  const showProgress = batchTotal > 0 && batchPending > 0;

  return (
    <div
      className="rounded-lg border border-stone-200 bg-white p-4 shadow-sm"
      data-testid={`memory-sync-card-${status.provider}`}>
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="font-medium text-stone-900">{label}</span>
            <span
              className={`rounded-md px-2 py-0.5 text-xs font-medium ${freshnessBadgeClass(status.freshness)}`}
              data-testid={`memory-sync-freshness-${status.provider}`}>
              {FRESHNESS_LABEL[status.freshness]}
            </span>
          </div>
          <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs text-stone-500">
            <span data-testid={`memory-sync-chunks-${status.provider}`}>
              {lifetime.toLocaleString()} chunks
            </span>
            {lastSync && <span>Last chunk {lastSync}</span>}
          </div>
        </div>
      </div>

      {showProgress && (
        <div className="mt-3" data-testid={`memory-sync-progress-${status.provider}`}>
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-stone-100">
            <div
              className="h-full bg-ocean-400 transition-all"
              style={{ width: `${pct}%` }}
              role="progressbar"
              aria-valuenow={batchProcessed}
              aria-valuemin={0}
              aria-valuemax={batchTotal}
            />
          </div>
          <div className="mt-1 text-xs text-stone-500">
            <span data-testid={`memory-sync-pending-${status.provider}`}>
              {batchProcessed.toLocaleString()} of {batchTotal.toLocaleString()} processed
            </span>
            {pending > 0 && (
              <span className="text-stone-400"> · {pending.toLocaleString()} pending</span>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

export function MemorySyncConnections({ pollIntervalMs }: MemorySyncConnectionsProps) {
  const [statuses, setStatuses] = useState<MemorySyncStatus[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const loadStatuses = useCallback(async () => {
    try {
      console.debug('[ui-flow][memory-sync] fetching status list');
      const list = await memorySyncStatusList();
      setStatuses(list);
      setLoadError(null);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.error('[ui-flow][memory-sync] status list failed', message);
      setLoadError(message);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      await loadStatuses();
      if (cancelled) return;
    })();
    return () => {
      cancelled = true;
    };
  }, [loadStatuses]);

  useEffect(() => {
    if (!pollIntervalMs) return undefined;
    const id = setInterval(() => {
      void loadStatuses();
    }, pollIntervalMs);
    return () => clearInterval(id);
  }, [pollIntervalMs, loadStatuses]);

  if (loading) {
    return (
      <section className="memory-sync-connections" data-testid="memory-sync-connections">
        <h3 className="text-sm font-semibold text-stone-700">Memory sources</h3>
        <p className="mt-2 text-xs text-stone-500">Loading…</p>
      </section>
    );
  }

  if (loadError) {
    return (
      <section className="memory-sync-connections" data-testid="memory-sync-connections">
        <h3 className="text-sm font-semibold text-stone-700">Memory sources</h3>
        <p className="mt-2 rounded-md bg-coral-50 p-2 text-xs text-coral-800 break-words">
          Failed to load sync status: {loadError}
        </p>
      </section>
    );
  }

  if (statuses.length === 0) {
    return (
      <section className="memory-sync-connections" data-testid="memory-sync-connections">
        <h3 className="text-sm font-semibold text-stone-700">Memory sources</h3>
        <p className="mt-2 text-xs text-stone-500">
          No content has been synced into memory yet. Connect an integration to start.
        </p>
      </section>
    );
  }

  return (
    <section className="memory-sync-connections" data-testid="memory-sync-connections">
      <h3 className="text-sm font-semibold text-stone-700">Memory sources</h3>
      <div className="mt-2 space-y-2">
        {statuses.map(s => (
          <SourceCard key={s.provider} status={s} />
        ))}
      </div>
    </section>
  );
}
