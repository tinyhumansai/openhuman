import { useEffect } from 'react';

import { reportMemoryQuarantine } from '../../lib/userErrors/report';
import { useCoreState } from '../../providers/CoreStateProvider';
import { useAppDispatch } from '../../store/hooks';
import { memoryTreePipelineStatus } from '../../utils/tauriCommands/memoryTree';

/**
 * How often the shell asks the core whether a corrupt-store quarantine is
 * outstanding. The condition changes on the order of a sync, not a keystroke,
 * and the status call is a handful of counters plus a directory walk.
 */
export const MEMORY_QUARANTINE_POLL_MS = 60_000;

/**
 * App-wide replay of the corrupt-store notice (openhuman#5820).
 *
 * The live `memory_store_corrupt` socket broadcast reaches only a renderer
 * that is connected when the quarantine happens; a boot-time integrity check
 * fires before that socket exists, and the broadcast has no replay. The
 * core derives the quarantine from disk in `memory_tree_pipeline_status`, so
 * polling it from the shell — not just from the Brain panel — puts the notice
 * in front of a user who never opens Brain, and retires it once they have
 * re-synced. Same descriptor id as the socket path, so the two never stack.
 */
export function useMemoryQuarantinePoll(): void {
  const dispatch = useAppDispatch();
  const { snapshot } = useCoreState();
  const isAuthenticated = snapshot.auth.isAuthenticated;

  useEffect(() => {
    if (!isAuthenticated) return;
    let cancelled = false;
    // A slow response must not overwrite a newer one: a delayed
    // `resynced: false` landing after a fresh `resynced: true` would re-open
    // a notice the user has already earned the retirement of.
    let latestRequest = 0;
    const tick = async () => {
      const request = ++latestRequest;
      try {
        const status = await memoryTreePipelineStatus();
        if (!cancelled && request === latestRequest) {
          reportMemoryQuarantine(dispatch, status.quarantine);
        }
      } catch (err) {
        // Signed out mid-poll, core restarting: nothing to report, try again
        // next tick.
        console.debug('[memory-quarantine-poll] status read failed: %o', err);
      }
    };
    void tick();
    const id = window.setInterval(() => void tick(), MEMORY_QUARANTINE_POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [dispatch, isAuthenticated]);
}
