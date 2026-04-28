import { useCallback, useEffect, useRef } from 'react';

import { useCoreState } from '../providers/CoreStateProvider';
import { userApi } from '../services/api/userApi';

/**
 * Hook to refetch the authoritative user state from the backend after a chat
 * turn finishes. Updates the global snapshot in CoreStateProvider.
 *
 * Includes a 750ms debounce to collapse multiple rapid turn-finalized events.
 */
export function useRefetchSnapshotOnTurnEnd() {
  const { patchSnapshot } = useCoreState();
  const debounceTimerRef = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (debounceTimerRef.current !== null) {
        window.clearTimeout(debounceTimerRef.current);
        debounceTimerRef.current = null;
      }
    };
  }, []);

  const refetch = useCallback(() => {
    if (debounceTimerRef.current !== null) {
      window.clearTimeout(debounceTimerRef.current);
    }

    debounceTimerRef.current = window.setTimeout(() => {
      debounceTimerRef.current = null;

      // Fire-and-forget on a microtask
      void (async () => {
        try {
          const user = await userApi.getMe();
          patchSnapshot({ currentUser: user });
        } catch (error) {
          console.warn('[useRefetchSnapshotOnTurnEnd] failed to refetch user state:', error);
        }
      })();
    }, 750);
  }, [patchSnapshot]);

  return { refetch };
}
