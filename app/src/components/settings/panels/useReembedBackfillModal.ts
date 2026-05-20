/**
 * #1574 §4b: advisory re-embed-backfill modal state, driven entirely by the
 * backend status RPC.
 *
 * After a settings save, the core's coverage-gated `ensure_reembed_backfill`
 * has already decided whether the embedder change needs a re-embed; this hook
 * just surfaces its progress (no fragile frontend "did the embedder change"
 * detection). Extracted from `AIPanel` so the logic is unit-testable in
 * isolation rather than only via a full 2000-line component render.
 */
import { useCallback, useEffect, useState } from 'react';

import { memoryTreeBackfillStatus } from '../../../utils/tauriCommands/memoryTree';

export interface ReembedState {
  open: boolean;
  pending: number;
}

export interface ReembedBackfillModal {
  reembed: ReembedState;
  /** Wrap a settings-save: persist, then (only on success) surface re-embed progress. */
  handleSave: () => Promise<void>;
  /** Close the advisory modal (re-embed continues in the background). */
  dismissReembed: () => void;
}

const POLL_MS = 2000;

/**
 * @param save Persists settings; resolves `true` only on a successful write
 *   (a failed save did not change the embedder, so nothing to surface).
 */
export function useReembedBackfillModal(save: () => Promise<boolean>): ReembedBackfillModal {
  const [reembed, setReembed] = useState<ReembedState>({ open: false, pending: 0 });

  const handleSave = useCallback(async () => {
    const ok = await save();
    if (!ok) return;
    try {
      const st = await memoryTreeBackfillStatus();
      if (st.in_progress) {
        setReembed({ open: true, pending: st.pending_jobs });
      }
    } catch (e) {
      console.warn('[ai-panel] backfill status check failed', e);
    }
  }, [save]);

  const dismissReembed = useCallback(() => setReembed({ open: false, pending: 0 }), []);

  useEffect(() => {
    if (!reembed.open) return;
    let cancelled = false;
    // Serialize polls — if a status call takes >POLL_MS, skip the next tick
    // rather than overlapping requests.
    let inFlight = false;
    const id = window.setInterval(() => {
      if (inFlight) return;
      inFlight = true;
      void (async () => {
        try {
          const st = await memoryTreeBackfillStatus();
          if (cancelled) return;
          if (!st.in_progress) {
            setReembed({ open: false, pending: 0 });
          } else {
            setReembed(r => ({ ...r, pending: st.pending_jobs }));
          }
        } catch (e) {
          console.warn('[ai-panel] backfill poll failed', e);
        } finally {
          inFlight = false;
        }
      })();
    }, POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [reembed.open]);

  return { reembed, handleSave, dismissReembed };
}
