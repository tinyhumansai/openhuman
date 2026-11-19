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
// The listener lifetime starts at the button click — before the system browser
// even opens — and the Rust shell hard-kills the loopback server when it
// elapses (`loopback_oauth.rs` `timeout(lifetime, run)`). A real GitHub/Google
// sign-in (account chooser, password manager, 2FA/OTP, first-time consent)
// routinely exceeds 60s; if the server dies first, the post-auth redirect lands
// on a dead port and the browser shows "127.0.0.1:<port> not reached" even
// though OAuth succeeded. 5 minutes comfortably covers an interactive sign-in.
const DEFAULT_TIMEOUT_SECS = 300;
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
 * Clean up the previous attempt before starting a new listener, including
 * its callback timeout so it cannot stop a newer listener after a retry.
 */
let activeTeardown: ((reason?: Error) => void) | null = null;
let listenerGeneration = 0;

/**
 * Start a one-shot loopback listener. Returns `null` if not running inside
 * Tauri, or if the shell fails to bind (port in use, etc) — the caller should
 * then fall back to the `openhuman://` deep-link redirect.
 */
export const startLoopbackOauthListener = async (
  options: StartLoopbackOptions = {}
): Promise<LoopbackHandle | null> => {
  const generation = ++listenerGeneration;
  if (activeTeardown) {
    const prev = activeTeardown;
    activeTeardown = null;
    prev(new Error('Loopback OAuth listener superseded by a new attempt'));
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
    if (generation !== listenerGeneration) return;
    try {
      await invoke('stop_loopback_oauth_listener');
    } catch (err) {
      console.warn('[loopback-oauth] stop failed', err);
    }
  };

  // `done` closes the race where the timeout (or a cancel) lands *before* the
  // async `listen()` registration resolves: the just-registered unlisten handle
  // is torn down instead of being stored in module-global `activeUnlisten`.
  let done = false;
  let timer: number | undefined;
  let unlisten: UnlistenFn | null = null;
  let supersededError: Error | null = null;
  let rejectCallback: ((error: Error) => void) | null = null;
  const teardown = (reason?: Error) => {
    if (reason) supersededError = reason;
    done = true;
    window.clearTimeout(timer);
    if (activeTeardown === teardown) activeTeardown = null;
    if (unlisten) {
      unlisten();
      unlisten = null;
    }
    if (reason) rejectCallback?.(reason);
    rejectCallback = null;
  };
  activeTeardown = teardown;

  const awaitCallback = (): Promise<string> =>
    new Promise<string>((resolve, reject) => {
      if (supersededError) {
        reject(supersededError);
        return;
      }
      rejectCallback = reject;
      timer = window.setTimeout(() => {
        teardown();
        void stop();
        reject(new Error('Loopback OAuth listener timed out'));
      }, timeoutSecs * 1000);

      listen<CallbackPayload>(CALLBACK_EVENT, event => {
        if (done) return;
        teardown();
        resolve(event.payload.url);
      })
        .then(fn => {
          if (done) {
            fn();
            return;
          }
          unlisten = fn;
        })
        .catch(err => {
          if (done) return;
          window.clearTimeout(timer);
          reject(err);
        });
    });

  // Cancel must also clear the timeout: a cancelled attempt's timer otherwise
  // fires later and stops whichever listener is active by then (a retry's).
  const cancel = async () => {
    teardown();
    await stop();
  };

  return { redirectUri: redirectUriWithState, state: result.state, awaitCallback, cancel };
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
