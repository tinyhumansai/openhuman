import { useCallback, useEffect, useRef, useState } from 'react';

import { memorySourcesList, type Source } from '../../../services/api/memoryApi';
import { type CoreCronJob, openhumanCronList } from '../../../utils/tauriCommands/cron';

/**
 * Aggregated, view-only snapshot of the background work the app runs on the
 * user's behalf: scheduled cron jobs and memory syncing/ingestion. Surfaced in {@link BackgroundInboxCard}
 * alongside the thread's detached sub-agents so users aren't oblivious to
 * background LLM activity.
 *
 * Everything here is read-only and only fetched while the panel is open — we
 * never poll in the background to keep this off the hot path.
 */

/** One synced memory source, reduced to what the activity card shows. */
export interface MemorySyncRow {
  provider: string;
  /** `active` while the source is syncing. */
  freshness: 'active' | 'idle';
}

/** Memory sources (Memory v2 Documents registry) and whether any is syncing. */
export interface MemorySyncSummary {
  /** True while at least one source is syncing. */
  ingesting: boolean;
  currentTitle?: string;
  /** How many sources are syncing right now. */
  queueDepth: number;
  providers: MemorySyncRow[];
}

interface BackgroundActivity {
  cronJobs: CoreCronJob[];
  memory: MemorySyncSummary;
  loading: boolean;
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
  const [memory, setMemory] = useState<MemorySyncSummary>(EMPTY_MEMORY);
  const [loading, setLoading] = useState(false);

  const cancelledRef = useRef(false);
  // Snapshot of "is anything live" so the poll loop can pick its cadence.
  const busyRef = useRef(false);

  const fetchOnce = useCallback(async () => {
    const [cronRes, sourcesRes] = await Promise.allSettled([
      openhumanCronList(),
      memorySourcesList(),
    ]);
    if (cancelledRef.current) return;

    if (cronRes.status === 'fulfilled') {
      setCronJobs(sortCronJobs(cronRes.value.result ?? []));
    } else {
      console.debug('[background-activity] cron_list failed: %o', cronRes.reason);
    }

    // Memory off (or no sources) answers with an error / empty list; either
    // way there is no memory activity to show.
    const sources: Source[] =
      sourcesRes.status === 'fulfilled' ? (sourcesRes.value.sources ?? []) : [];
    if (sourcesRes.status === 'rejected') {
      console.debug('[background-activity] memory_sources_list failed: %o', sourcesRes.reason);
    }
    const syncing = sources.filter(source => source.status === 'syncing');
    setMemory({
      ingesting: syncing.length > 0,
      currentTitle: syncing[0] ? syncing[0].label || syncing[0].target : undefined,
      queueDepth: syncing.length,
      providers: sources.map(source => ({
        provider: source.label || source.target,
        freshness: source.status === 'syncing' ? 'active' : 'idle',
      })),
    });

    // Fast-poll only while a source is genuinely syncing.
    busyRef.current = syncing.length > 0;

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

  return { cronJobs, memory, loading };
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
