import { useCallback, useEffect, useRef, useState } from 'react';

import { callCoreRpc } from '../../../services/coreRpcClient';
import { isTauri } from '../../../utils/tauriCommands/common';
import { type CoreCronJob, openhumanCronList } from '../../../utils/tauriCommands/cron';
import {
  memorySyncStatusList,
  type MemorySyncStatusRow,
} from '../../../utils/tauriCommands/memoryTree';
import {
  subconsciousStatus,
  subconsciousTriggersStatus,
} from '../../../utils/tauriCommands/subconscious';

/**
 * Aggregated, view-only snapshot of the background work the app runs on the
 * user's behalf: scheduled cron jobs, the subconscious/heartbeat loop, and
 * memory syncing/ingestion. Surfaced in {@link BackgroundProcessesPanel}
 * alongside the thread's detached sub-agents so users aren't oblivious to
 * background LLM activity.
 *
 * Everything here is read-only and only fetched while the panel is open — we
 * never poll in the background to keep this off the hot path.
 */

/** Distilled subconscious status for the panel row. */
export interface SubconsciousSummary {
  enabled: boolean;
  mode: string;
  lastTickAt: number | null;
  totalTicks: number;
  /** Live indicator — orchestrator running or work queued right now. */
  working: boolean;
  queueDepth: number | null;
}

/** Memory worker + per-provider freshness rows. */
export interface MemorySyncSummary {
  /** True while the ingestion worker is processing a document. */
  ingesting: boolean;
  currentTitle?: string;
  queueDepth: number;
  providers: MemorySyncStatusRow[];
}

export interface BackgroundActivity {
  cronJobs: CoreCronJob[];
  subconscious: SubconsciousSummary | null;
  memory: MemorySyncSummary;
  loading: boolean;
}

interface IngestionStatusEnvelope {
  running: boolean;
  current_title?: string;
  queue_depth: number;
}

const EMPTY_MEMORY: MemorySyncSummary = { ingesting: false, queueDepth: 0, providers: [] };

const DEFAULT_POLL_MS = 5000;
const FAST_POLL_MS = 2000;

/** Soonest-first, enabled jobs ahead of paused ones. */
function sortCronJobs(jobs: CoreCronJob[]): CoreCronJob[] {
  return [...jobs].sort((a, b) => {
    if (a.enabled !== b.enabled) return a.enabled ? -1 : 1;
    return Date.parse(a.next_run) - Date.parse(b.next_run);
  });
}

/**
 * Fetch + adaptive-poll the background activity snapshot, but only while
 * `open` is true. Also refreshes immediately on `openhuman:memory-sync-stage`
 * window events (dispatched globally by `socketService` from the core's
 * `memory:sync_stage` socket event) so the memory section reacts live.
 */
export function useBackgroundActivity(open: boolean): BackgroundActivity {
  const [cronJobs, setCronJobs] = useState<CoreCronJob[]>([]);
  const [subconscious, setSubconscious] = useState<SubconsciousSummary | null>(null);
  const [memory, setMemory] = useState<MemorySyncSummary>(EMPTY_MEMORY);
  const [loading, setLoading] = useState(false);

  const cancelledRef = useRef(false);
  // Snapshot of "is anything live" so the poll loop can pick its cadence.
  const busyRef = useRef(false);

  const fetchOnce = useCallback(async () => {
    if (!isTauri()) {
      // Non-Tauri / dev preview: nothing to surface, just stop the spinner.
      setLoading(false);
      return;
    }

    const [cronRes, subRes, trigRes, ingestRes, providerRes] = await Promise.allSettled([
      openhumanCronList(),
      subconsciousStatus(),
      subconsciousTriggersStatus(),
      callCoreRpc<IngestionStatusEnvelope>({ method: 'openhuman.memory_ingestion_status' }),
      memorySyncStatusList(),
    ]);
    if (cancelledRef.current) return;

    if (cronRes.status === 'fulfilled') {
      setCronJobs(sortCronJobs(cronRes.value.result ?? []));
    } else {
      console.debug('[background-activity] cron_list failed: %o', cronRes.reason);
    }

    if (subRes.status === 'fulfilled') {
      const s = subRes.value.result;
      const trig = trigRes.status === 'fulfilled' ? trigRes.value.result : null;
      const queueDepth = trig?.queue_depth ?? null;
      setSubconscious({
        enabled: s.enabled,
        mode: s.mode,
        lastTickAt: s.last_tick_at,
        totalTicks: s.total_ticks,
        working: Boolean(trig?.orchestrator_running) || (queueDepth ?? 0) > 0,
        queueDepth,
      });
    } else {
      console.debug('[background-activity] subconscious_status failed: %o', subRes.reason);
    }

    const ingest = ingestRes.status === 'fulfilled' ? ingestRes.value : null;
    const providers = providerRes.status === 'fulfilled' ? providerRes.value : [];
    if (ingestRes.status === 'rejected') {
      console.debug('[background-activity] ingestion_status failed: %o', ingestRes.reason);
    }
    setMemory({
      ingesting: Boolean(ingest?.running),
      currentTitle: ingest?.current_title,
      queueDepth: ingest?.queue_depth ?? 0,
      providers,
    });

    // Only consider work *genuinely live* for the fast-poll cadence: the
    // ingestion worker actually running/queued, or a provider with a fresh
    // chunk (<30s). A stale, un-drained embedding wave (batch_total >
    // batch_processed but idle freshness) must NOT pin us to fast-poll.
    busyRef.current =
      Boolean(ingest?.running) ||
      (ingest?.queue_depth ?? 0) > 0 ||
      providers.some(p => p.freshness === 'active');

    setLoading(false);
  }, []);

  // Poll loop, gated entirely on `open`.
  useEffect(() => {
    if (!open) return;
    cancelledRef.current = false;
    setLoading(true);
    let timer: ReturnType<typeof setTimeout> | null = null;

    const tick = async () => {
      await fetchOnce();
      if (cancelledRef.current) return;
      timer = setTimeout(tick, busyRef.current ? FAST_POLL_MS : DEFAULT_POLL_MS);
    };
    void tick();

    return () => {
      cancelledRef.current = true;
      if (timer) clearTimeout(timer);
    };
  }, [open, fetchOnce]);

  // Live refresh on memory sync stage changes while the panel is open.
  useEffect(() => {
    if (!open) return;
    const onStage = () => {
      void fetchOnce();
    };
    window.addEventListener('openhuman:memory-sync-stage', onStage);
    return () => window.removeEventListener('openhuman:memory-sync-stage', onStage);
  }, [open, fetchOnce]);

  return { cronJobs, subconscious, memory, loading };
}

/** Stages that mean a sync has settled for a given source. */
const TERMINAL_STAGES = new Set(['completed', 'failed']);

/**
 * Poll-free "is any memory sync in flight right now" signal, driven purely by
 * the `openhuman:memory-sync-stage` window events that `socketService`
 * dispatches. Cheap enough to keep mounted while the panel is closed so the
 * background-activity badge can light up for live syncing without polling.
 */
export function useMemorySyncActive(): boolean {
  const [active, setActive] = useState(false);
  const activeIdsRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    const onStage = (e: Event) => {
      const data = (e as CustomEvent).detail as {
        stage?: string;
        source_id?: string | null;
        connection_id?: string | null;
      };
      const rowId = data?.source_id ?? data?.connection_id ?? 'unknown';
      const stage = data?.stage ?? '';
      const ids = activeIdsRef.current;
      if (TERMINAL_STAGES.has(stage)) ids.delete(rowId);
      else ids.add(rowId);
      setActive(ids.size > 0);
    };
    window.addEventListener('openhuman:memory-sync-stage', onStage);
    return () => window.removeEventListener('openhuman:memory-sync-stage', onStage);
  }, []);

  return active;
}
