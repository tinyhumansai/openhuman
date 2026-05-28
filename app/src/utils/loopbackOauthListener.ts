import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

import { E2E_RESTART_APP_AS_RELOAD } from './config';
import { isTauri } from './tauriCommands/common';

/**
 * Loopback OAuth listener — preferred desktop redirect target ahead of
 * `openhuman://` deep links (RFC 8252).
 *
 * The Tauri shell binds `http://127.0.0.1:<port>/auth` on demand, returns the
 * redirect URI plus a state nonce, and emits a `loopback-oauth-callback` event
 * once the backend redirects the browser back. Callers append the state to the
 * URL handed to the backend so a hostile page on the same loopback origin
 * cannot fake a callback.
 *
 * Falls back gracefully: any failure (not in Tauri, port already in use,
 * timeout) returns `null` so callers can take the `openhuman://` deep-link
 * path instead.
 */

const DEFAULT_PORT = 53824;
const DEFAULT_TIMEOUT_SECS = 60;
const CALLBACK_EVENT = 'loopback-oauth-callback';

export interface LoopbackHandle {
  /** Fully qualified redirect URI to give to the backend, state already appended. */
  redirectUri: string;
  /** State nonce the backend must echo back as `?state=<value>`. */
  state: string;
  /** Resolves with the full callback URL once the browser hits the loopback. */
  awaitCallback: () => Promise<string>;
  /** Tear down the listener early (e.g. user cancelled). */
  cancel: () => Promise<void>;
}

interface StartResult {
  redirectUri: string;
  state: string;
}

interface CallbackPayload {
  url: string;
}

export interface StartLoopbackOptions {
  /** Loopback port to bind. Must be pre-registered with the backend. */
  port?: number;
  /** How long to keep the listener alive. */
  timeoutSecs?: number;
}

/**
 * The JS-side `listen()` handler from a previous call. We unsubscribe it
 * before starting a new listener so a single Rust emit can't fan out to
 * multiple stale handlers (happens when the user re-clicks before the
 * previous OAuth round-trip completes).
 */
let activeUnlisten: UnlistenFn | null = null;

/**
 * Start a one-shot loopback listener. Returns `null` if not running inside
 * Tauri, or if the shell fails to bind (port in use, etc) — the caller should
 * then fall back to the `openhuman://` deep-link redirect.
 */
export const startLoopbackOauthListener = async (
  options: StartLoopbackOptions = {}
): Promise<LoopbackHandle | null> => {
  if (activeUnlisten) {
    const prev = activeUnlisten;
    activeUnlisten = null;
    prev();
  }
  if (!isTauri()) {
    return null;
  }

  const port = options.port ?? DEFAULT_PORT;
  const timeoutSecs = options.timeoutSecs ?? DEFAULT_TIMEOUT_SECS;

  let result: StartResult;
  try {
    result = await invoke<StartResult>('start_loopback_oauth_listener', { port, timeoutSecs });
  } catch (err) {
    console.warn('[loopback-oauth] start failed, falling back to deep link', err);
    return null;
  }

  const redirectUriWithState = appendState(result.redirectUri, result.state);

  const stop = async () => {
    try {
      await invoke('stop_loopback_oauth_listener');
    } catch (err) {
      console.warn('[loopback-oauth] stop failed', err);
    }
  };

  const awaitCallback = (): Promise<string> =>
    new Promise<string>((resolve, reject) => {
      // `timedOut` closes the race where `setTimeout` fires *before* the async
      // `listen()` registration resolves: previously the just-registered
      // unlisten handle was stored in module-global `activeUnlisten` after the
      // promise had already rejected, leaving the listener armed until the
      // next `startLoopbackOauthListener` call cleaned it up.
      let timedOut = false;
      let unlisten: UnlistenFn | null = null;
      const timer = window.setTimeout(() => {
        timedOut = true;
        if (unlisten) {
          unlisten();
          if (activeUnlisten === unlisten) activeUnlisten = null;
        }
        void stop();
        reject(new Error('Loopback OAuth listener timed out'));
      }, timeoutSecs * 1000);

      listen<CallbackPayload>(CALLBACK_EVENT, event => {
        if (timedOut) return;
        window.clearTimeout(timer);
        if (unlisten) {
          unlisten();
          if (activeUnlisten === unlisten) activeUnlisten = null;
        }
        resolve(event.payload.url);
      })
        .then(fn => {
          if (timedOut) {
            // Timer already rejected the promise — tear down the
            // just-registered handle so it does not leak into
            // `activeUnlisten` and stay armed past the timeout.
            fn();
            return;
          }
          unlisten = fn;
          activeUnlisten = fn;
        })
        .catch(err => {
          if (timedOut) return;
          window.clearTimeout(timer);
          reject(err);
        });
    });

  return { redirectUri: redirectUriWithState, state: result.state, awaitCallback, cancel: stop };
};

const appendState = (uri: string, state: string): string => {
  const separator = uri.includes('?') ? '&' : '?';
  return `${uri}${separator}state=${encodeURIComponent(state)}`;
};

// E2E hook: expose the same listener factory the production OAuth button uses
// so spec helpers can drive the real loopback flow (Rust HTTP server + event
// emit + frontend listener) without scripting the OAuth button UI itself.
// Gated on the E2E-mode VITE flag baked in by app/scripts/e2e-build.sh so it
// never leaks into release bundles.
if (typeof window !== 'undefined' && E2E_RESTART_APP_AS_RELOAD) {
  type WithE2eHook = Window & { __startLoopbackOauthListener?: typeof startLoopbackOauthListener };
  (window as WithE2eHook).__startLoopbackOauthListener = startLoopbackOauthListener;
}
