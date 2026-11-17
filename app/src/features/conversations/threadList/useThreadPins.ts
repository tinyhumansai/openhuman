import debugFactory from 'debug';
import { useCallback, useState } from 'react';

import { threadApi } from '../../../services/api/threadApi';
import { useAppDispatch } from '../../../store/hooks';
import { loadThreads } from '../../../store/threadSlice';
import type { Thread } from '../../../types/thread';
import { isThreadPinned, labelsWithPin } from './groupThreads';

const debug = debugFactory('conversations:pins');

/**
 * Pin / unpin a thread through the core's label store.
 *
 * The pin is the reserved `pinned` label, written with
 * `openhuman.threads_update_labels` so it persists with the thread. The row
 * moves immediately via a local override, held until the thread list reload
 * carries the persisted labels; a failed write drops the override, so the row
 * snaps back to what the core actually stored.
 */
export function useThreadPins(): {
  isPinned: (thread: Thread) => boolean;
  togglePin: (thread: Thread, pinned: boolean) => void;
} {
  const dispatch = useAppDispatch();
  const [overrides, setOverrides] = useState<ReadonlyMap<string, boolean>>(() => new Map());

  const isPinned = useCallback(
    (thread: Thread) => overrides.get(thread.id) ?? isThreadPinned(thread),
    [overrides]
  );

  const togglePin = useCallback(
    (thread: Thread, pinned: boolean) => {
      setOverrides(prev => new Map(prev).set(thread.id, pinned));
      const clearOverride = () =>
        setOverrides(prev => {
          if (!prev.has(thread.id)) return prev;
          const next = new Map(prev);
          next.delete(thread.id);
          return next;
        });
      debug('toggle pin thread=%s pinned=%s', thread.id, pinned);
      void threadApi
        .updateLabels(thread.id, labelsWithPin(thread.labels, pinned))
        .then(() => dispatch(loadThreads()).unwrap())
        .catch((error: unknown) => {
          debug(
            'pin update failed thread=%s: %s',
            thread.id,
            error instanceof Error ? error.message : String(error)
          );
        })
        .finally(clearOverride);
    },
    [dispatch]
  );

  return { isPinned, togglePin };
}
