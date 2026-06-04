/**
 * usePttHotkey
 *
 * Subscribes the configured push-to-talk shortcut to the Tauri shell whenever
 * the persisted `shortcut` field on the `ptt` slice changes. Resets the
 * transient `isHeld` flag on mount so a stale rehydrated value (left over from
 * a crash mid-press) can never leave the UI thinking the PTT key is held.
 *
 * Wired into the renderer once via `PttHotkeyManager` (T11), mounted in
 * `App.tsx` alongside the dictation manager.
 */
import { useEffect } from 'react';
import { useDispatch, useSelector } from 'react-redux';

import { selectPttShortcut, setIsHeld, setPttRegistrationError } from '../store/pttSlice';
import { registerPttHotkey, unregisterPttHotkey } from '../utils/tauriCommands/ptt';

export function usePttHotkey(): void {
  const dispatch = useDispatch();
  const shortcut = useSelector(selectPttShortcut);

  // Clear the transient isHeld flag on mount — a crash mid-press could
  // otherwise rehydrate to "held forever".
  useEffect(() => {
    dispatch(setIsHeld(false));
  }, [dispatch]);

  useEffect(() => {
    let cancelled = false;
    const apply = async () => {
      try {
        if (shortcut && shortcut.trim().length > 0) {
          await registerPttHotkey(shortcut);
          if (!cancelled) dispatch(setPttRegistrationError(null));
        } else {
          await unregisterPttHotkey();
          if (!cancelled) dispatch(setPttRegistrationError(null));
        }
      } catch (err) {
        if (!cancelled) {
          const msg = err instanceof Error ? err.message : String(err);
          console.warn('[ptt] hotkey (un)register failed', err);
          dispatch(setPttRegistrationError(msg));
        }
      }
    };
    void apply();
    return () => {
      cancelled = true;
    };
  }, [shortcut, dispatch]);
}
