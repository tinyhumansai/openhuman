import { useEffect } from 'react';

import { clearToken, setAuthBootstrapComplete, setToken } from '../store/authSlice';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import {
  getAuthState,
  getSessionToken,
  isTauri,
  openhumanWorkspaceOnboardingFlagSet,
} from '../utils/tauriCommands';

const AUTH_BOOTSTRAP_TIMEOUT_MS = 5000;

async function withTimeout<T>(promise: Promise<T>, timeoutMs: number): Promise<T> {
  let timeoutId: ReturnType<typeof setTimeout> | null = null;
  const timeoutPromise = new Promise<never>((_, reject) => {
    timeoutId = setTimeout(() => reject(new Error(`timeout after ${timeoutMs}ms`)), timeoutMs);
  });

  try {
    return await Promise.race([promise, timeoutPromise]);
  } finally {
    if (timeoutId) clearTimeout(timeoutId);
  }
}

/**
 * UserProvider bootstraps auth token from core session state.
 */
const UserProvider = ({ children }: { children: React.ReactNode }) => {
  const dispatch = useAppDispatch();
  const token = useAppSelector(state => state.auth.token);
  const isAuthBootstrapComplete = useAppSelector(state => state.auth.isAuthBootstrapComplete);

  useEffect(() => {
    if (isAuthBootstrapComplete) return;

    let mounted = true;
    void (async () => {
      if (!isTauri()) {
        if (mounted) dispatch(setAuthBootstrapComplete(true));
        return;
      }

      try {
        const [authState, sessionToken] = await withTimeout(
          Promise.all([getAuthState(), getSessionToken()]),
          AUTH_BOOTSTRAP_TIMEOUT_MS
        );
        if (!mounted) return;

        if (authState.is_authenticated && sessionToken) {
          if (sessionToken !== token) {
            dispatch(setToken(sessionToken));
          }
        } else if (!authState.is_authenticated && token) {
          await dispatch(clearToken());
          try {
            await openhumanWorkspaceOnboardingFlagSet(false);
          } catch {
            // Best-effort: flag clear failure shouldn't block auth recovery
          }
        }
      } catch (err) {
        console.warn('[auth] Failed to restore session token from core RPC:', err);
      } finally {
        if (mounted) dispatch(setAuthBootstrapComplete(true));
      }
    })();

    return () => {
      mounted = false;
    };
  }, [token, dispatch, isAuthBootstrapComplete]);

  return <>{children}</>;
};

export default UserProvider;
