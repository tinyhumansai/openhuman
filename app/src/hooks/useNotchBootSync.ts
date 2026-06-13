import debug from 'debug';
import { useEffect, useRef } from 'react';

import { openhumanGetVoiceServerSettings, syncNotchVisibility } from '../utils/tauriCommands';

const log = debug('notch:boot');

/**
 * Sync the notch indicator to the persisted always-on listening state once the
 * core is ready — runs **once per boot**.
 *
 * The notch is the always-on listening HUD, so it stays hidden unless always-on
 * listening is enabled; it is no longer auto-shown unconditionally by the Tauri
 * shell (see `notch_window` / `dispatch_notch_on_main`). Failures are swallowed
 * (debug-logged) — notch visibility is cosmetic and must never block boot.
 */
export function useNotchBootSync(isBootstrapping: boolean): void {
  const notchSyncedRef = useRef(false);
  useEffect(() => {
    if (isBootstrapping || notchSyncedRef.current) return;
    notchSyncedRef.current = true;
    void (async () => {
      try {
        const res = await openhumanGetVoiceServerSettings();
        await syncNotchVisibility(res.result.always_on_enabled);
      } catch (err) {
        log('boot visibility sync failed: %o', err);
      }
    })();
  }, [isBootstrapping]);
}
