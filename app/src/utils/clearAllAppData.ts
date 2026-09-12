import { persistor } from '../store';
import { resetOpenHumanDataAndRestartCore, restartApp } from './tauriCommands';

const ACTIVE_USER_KEY = 'OPENHUMAN_ACTIVE_USER_ID';

/**
 * Selectively purge localStorage keys belonging to a single user.
 *
 * Removes:
 *  - `${userId}:persist:*`  — per-user Redux-persist blobs
 *  - `${userId}:*`          — any other user-scoped keys
 *  - `OPENHUMAN_ACTIVE_USER_ID` — the boot-time user seed (only when a userId
 *                                  is supplied so we don't wipe it on pre-login
 *                                  recovery where userId is null)
 *
 * Intentionally leaves other users' scoped keys untouched so that
 * "clear my data" on account B does not silently destroy account A's
 * persisted state (#983).
 */
function clearUserScopedStorage(userId: string | null): void {
  try {
    if (userId) {
      const prefix = `${userId}:`;
      const toRemove: string[] = [];
      for (let i = 0; i < localStorage.length; i++) {
        const key = localStorage.key(i);
        if (key && key.startsWith(prefix)) {
          toRemove.push(key);
        }
      }
      for (const key of toRemove) {
        localStorage.removeItem(key);
      }
      localStorage.removeItem(ACTIVE_USER_KEY);
    } else {
      // No known user (pre-login recovery) — fall back to clearing everything
      // so we don't leave orphaned blobs with no way to scope the deletion.
      localStorage.clear();
    }
  } catch (err) {
    console.warn('[clearAllAppData] storage clear failed:', err);
  } finally {
    try {
      sessionStorage.clear();
    } catch {
      // best-effort
    }
  }
}

interface ClearAllAppDataOptions {
  // Optional core-side session clear (e.g. `auth_clear_session`). Best-effort —
  // skipped silently if the caller cannot/does not provide it (e.g. pre-login
  // recovery from a corrupt key file, where there is no live session).
  clearSession?: () => Promise<unknown>;
  // User scope passed to the core reset so only the active account is deleted.
  userId?: string | null;
}

/**
 * Sign out + wipe every local data store and restart the app:
 *
 *  1. Best-effort `clearSession` to drop the core's auth state.
 *  2. Reset the openhuman workspace dir + restart the core sidecar.
 *  3. Purge redux-persist + window storage.
 *  4. Restart the desktop shell into the cleared session.
 *
 * Used by Settings (Danger Zone) and the Welcome screen's decryption-recovery
 * action. Throws on the first step that can't be recovered from — callers are
 * expected to surface that to the user.
 */
export const clearAllAppData = async ({
  clearSession,
  userId = null,
}: ClearAllAppDataOptions = {}): Promise<void> => {
  // 1. Best-effort core-side session clear. If the core is wedged or there is
  //    no session yet (pre-login recovery), keep going — we still want to wipe
  //    local data.
  if (clearSession) {
    try {
      await clearSession();
    } catch (err) {
      console.warn('[clearAllAppData] core session clear failed:', err);
    }
  }

  // 2. Delete the signed-in user's data dir + restart core. We pass `userId`
  //    explicitly: step 1's `clearSession()` already ran `auth_clear_session`,
  //    which removes the `active_user.toml` marker. If the reset resolved its
  //    target from that (now-absent) marker it would fall back to the pre-login
  //    `users/local` dir and delete an empty directory — leaving the real
  //    user's memory, sources, conversations, and history under `users/<id>`
  //    fully intact. That marker/ordering gap is the root cause of #4950
  //    ("Clear App Data does nothing"). Passing the id the caller already holds
  //    pins the deletion to the correct user regardless of marker state.
  await resetOpenHumanDataAndRestartCore(userId);

  // 3. Purge redux-persist + browser storage. `persistor.purge()` wipes the
  //    persisted backend; `clearUserScopedStorage` removes only the active
  //    user's localStorage keys so other accounts' data is not destroyed.
  await persistor.purge();
  clearUserScopedStorage(userId);

  // 4. Full app restart into the fresh pre-login session.
  await restartApp();
};
