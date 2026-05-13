import { persistor } from '../store';
import {
  resetOpenHumanDataAndRestartCore,
  restartApp,
  scheduleCefProfilePurge,
} from './tauriCommands';

export interface ClearAllAppDataOptions {
  // Optional core-side session clear (e.g. `auth_clear_session`). Best-effort —
  // skipped silently if the caller cannot/does not provide it (e.g. pre-login
  // recovery from a corrupt key file, where there is no live session).
  clearSession?: () => Promise<unknown>;
  // User scope passed to the CEF profile purge so per-user browser data is
  // queued for deletion on the next launch. `null` purges the unauthenticated
  // default profile.
  userId?: string | null;
}

/**
 * Sign out + wipe every local data store and restart the app:
 *
 *  1. Queue the CEF profile directory for deletion on next launch.
 *  2. Best-effort `clearSession` to drop the core's auth state.
 *  3. Reset the openhuman workspace dir + restart the core sidecar.
 *  4. Purge redux-persist + window storage.
 *  5. Restart the desktop shell so CEF reboots into the fresh profile.
 *
 * Used by Settings (Danger Zone) and the Welcome screen's decryption-recovery
 * action. Throws on the first step that can't be recovered from — callers are
 * expected to surface that to the user.
 */
export const clearAllAppData = async ({
  clearSession,
  userId = null,
}: ClearAllAppDataOptions = {}): Promise<void> => {
  // 1. Queue the active user-scoped CEF profile for deletion on next launch.
  //    The CEF process may still hold SQLite/cache handles, so we delete
  //    after the shell restarts.
  try {
    await scheduleCefProfilePurge(userId);
  } catch (err) {
    console.warn('[clearAllAppData] Failed to queue CEF profile purge:', err);
  }

  // 2. Best-effort core-side session clear. If the core is wedged or there is
  //    no session yet (pre-login recovery), keep going — we still want to wipe
  //    local data.
  if (clearSession) {
    try {
      await clearSession();
    } catch (err) {
      console.warn('[clearAllAppData] core session clear failed:', err);
    }
  }

  // 3. Delete workspace folder + restart core. The core RPC removes both the
  //    active openhuman_dir and the default `~/.openhuman`, then we restart
  //    the sidecar so it boots from a clean slate.
  await resetOpenHumanDataAndRestartCore();

  // 4. Purge redux-persist + browser storage. `persistor.purge()` wipes the
  //    persisted backend; localStorage/sessionStorage clear everything else
  //    (auth flags, theme, etc.).
  await persistor.purge();
  window.localStorage.clear();
  window.sessionStorage.clear();

  // 5. Full app restart so CEF reboots into the fresh pre-login profile.
  await restartApp();
};
