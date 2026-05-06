/**
 * App auto-update hook.
 *
 * Owns:
 *  - the state machine for the Tauri shell updater
 *    (idle | checking | available | downloading | ready_to_install |
 *     installing | restarting | up_to_date | error)
 *  - listeners on the `app-update:status` + `app-update:progress` events
 *    emitted by the Rust download/install commands
 *  - an opt-in auto-check cadence: one probe shortly after launch, then
 *    a periodic re-probe while the app stays open
 *  - an opt-in auto-download: when a check reports "available", the hook
 *    automatically calls `download_app_update` so the user only sees a
 *    "Restart to apply" prompt — never a "click to start downloading" one
 *
 * Pairs with the Rust side in `app/src-tauri/src/lib.rs` (`check_app_update`,
 * `download_app_update`, `install_app_update`). See `docs/AUTO_UPDATE.md`.
 */
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { useCallback, useEffect, useRef, useState } from 'react';

import {
  applyAppUpdate,
  type AppUpdateInfo,
  checkAppUpdate,
  downloadAppUpdate,
  installAppUpdate,
  isTauri,
} from '../utils/tauriCommands';

/** Phases driven by `app-update:status`, plus locally-derived ones. */
export type AppUpdatePhase =
  | 'idle'
  | 'checking'
  | 'available'
  | 'downloading'
  | 'ready_to_install'
  | 'installing'
  | 'restarting'
  | 'up_to_date'
  | 'error';

export interface AppUpdateProgress {
  /** Bytes received in the latest chunk callback. */
  chunk: number;
  /** Total bytes (null when the manifest didn't advertise a content-length). */
  total: number | null;
}

export interface UseAppUpdateOptions {
  /**
   * Run an automatic check shortly after the hook mounts.
   * Default: true. Skipped when `isTauri()` is false.
   */
  autoCheck?: boolean;
  /** Delay before the first auto-check fires, in ms. Default: 5_000. */
  initialCheckDelayMs?: number;
  /**
   * Repeat interval between background checks, in ms. Default: 15 * 60 * 1000.
   * Set to 0 (or a negative number) to disable repeating.
   */
  recheckIntervalMs?: number;
  /**
   * When a check reports an available update, automatically start the
   * download in the background so the user is only ever prompted to
   * restart. Default: true.
   */
  autoDownload?: boolean;
}

export interface UseAppUpdateResult {
  phase: AppUpdatePhase;
  /** Last successful check result (current/available versions, body). */
  info: AppUpdateInfo | null;
  /** Bytes downloaded so far (sum of every `app-update:progress` chunk this run). */
  bytesDownloaded: number;
  /** Latest `total` reported by the updater (may stay null). */
  totalBytes: number | null;
  /** Last error message, if any phase landed on `error`. */
  error: string | null;
  /** Manually run a check (does not download). */
  check: () => Promise<AppUpdateInfo | null>;
  /**
   * Start a background download. Normally called automatically when a check
   * reports an available update; exposed so callers can retry on error.
   */
  download: () => Promise<void>;
  /**
   * Install previously-downloaded bytes and restart. Never resolves on
   * success (the process exits mid-await). Falls back to {@link apply}
   * if no download has been staged.
   */
  install: () => Promise<void>;
  /**
   * Legacy combined download+install+restart. Prefer the auto-download flow
   * above; kept for callers that want a single explicit "do everything"
   * action.
   */
  apply: () => Promise<void>;
  /** Reset transient state (error, downloaded bytes) without changing `info`. */
  reset: () => void;
}

const DEFAULT_INITIAL_DELAY_MS = 5_000;
const DEFAULT_RECHECK_INTERVAL_MS = 15 * 60 * 1000; // 15m

/** A short grace before the auto-download fires, so the UI can show the
 *  fact that an update was *detected* (briefly) before going into "downloading"
 *  state. Cosmetic, not load-bearing. */
const AUTO_DOWNLOAD_GRACE_MS = 1_000;

/**
 * Translate a raw `app-update:status` payload into our phase enum, defaulting
 * to `error` for any unrecognized string so we don't silently swallow a bad
 * payload from the Rust side.
 */
function parseStatusPayload(raw: unknown): AppUpdatePhase {
  if (raw === 'checking') return 'checking';
  if (raw === 'downloading') return 'downloading';
  if (raw === 'ready_to_install') return 'ready_to_install';
  if (raw === 'installing') return 'installing';
  if (raw === 'restarting') return 'restarting';
  if (raw === 'up_to_date') return 'up_to_date';
  if (raw === 'error') return 'error';
  console.warn('[app-update] hook: unknown status payload', raw);
  return 'error';
}

export function useAppUpdate(options: UseAppUpdateOptions = {}): UseAppUpdateResult {
  const {
    autoCheck = true,
    initialCheckDelayMs = DEFAULT_INITIAL_DELAY_MS,
    recheckIntervalMs = DEFAULT_RECHECK_INTERVAL_MS,
    autoDownload = true,
  } = options;

  const [phase, setPhase] = useState<AppUpdatePhase>('idle');
  const [info, setInfo] = useState<AppUpdateInfo | null>(null);
  const [bytesDownloaded, setBytesDownloaded] = useState(0);
  const [totalBytes, setTotalBytes] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Refs to keep callbacks stable + survive React 18 strict-mode double-invoke.
  const mountedRef = useRef(true);
  const phaseRef = useRef<AppUpdatePhase>(phase);
  phaseRef.current = phase;
  // Tracks whether we've already kicked off a download for the current
  // `available` detection so the auto-download effect doesn't loop on
  // re-renders.
  const downloadInFlightRef = useRef(false);
  // Tracks whether bytes have been staged successfully. `install()` checks
  // this so it can fall back to the legacy combined apply path if the user
  // reaches "install" without a prior download (e.g. error mid-flow).
  const stagedRef = useRef(false);

  /** Probe the updater endpoint. Does not download. */
  const check = useCallback(async (): Promise<AppUpdateInfo | null> => {
    if (!isTauri()) {
      console.debug('[app-update] hook.check: skipped — not running in Tauri');
      return null;
    }
    if (
      phaseRef.current === 'downloading' ||
      phaseRef.current === 'installing' ||
      phaseRef.current === 'ready_to_install'
    ) {
      console.debug('[app-update] hook.check: skipped — flow in progress', phaseRef.current);
      return null;
    }
    console.debug('[app-update] hook.check: starting');
    setPhase('checking');
    setError(null);
    try {
      const result = await checkAppUpdate();
      if (!mountedRef.current) return result;
      if (result?.available) {
        console.info(
          `[app-update] hook.check: update available ${result.current_version} -> ${result.available_version}`
        );
        setInfo(result);
        setPhase('available');
      } else {
        console.debug('[app-update] hook.check: up to date');
        setInfo(result);
        setPhase('up_to_date');
      }
      return result;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.warn('[app-update] hook.check: failed', message);
      if (mountedRef.current) {
        setError(message);
        setPhase('error');
      }
      return null;
    }
  }, []);

  /** Download bytes in the background. Normally fires automatically. */
  const download = useCallback(async (): Promise<void> => {
    if (!isTauri()) {
      console.debug('[app-update] hook.download: skipped — not running in Tauri');
      return;
    }
    if (downloadInFlightRef.current) {
      console.debug('[app-update] hook.download: already in flight');
      return;
    }
    downloadInFlightRef.current = true;
    stagedRef.current = false;
    setBytesDownloaded(0);
    setTotalBytes(null);
    setError(null);
    console.info('[app-update] hook.download: starting');
    try {
      const result = await downloadAppUpdate();
      if (!mountedRef.current) return;
      if (result?.ready) {
        stagedRef.current = true;
        console.info(`[app-update] hook.download: staged ${result.version}`);
        // The Rust side has already emitted `ready_to_install`. The status
        // listener will move us into that phase; nothing else to do here.
      } else {
        console.debug('[app-update] hook.download: nothing to download');
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.error('[app-update] hook.download: failed', message);
      if (mountedRef.current) {
        setError(message);
        setPhase('error');
      }
    } finally {
      downloadInFlightRef.current = false;
    }
  }, []);

  /** Install the staged bytes and restart. Falls back to `apply()` if nothing is staged. */
  const install = useCallback(async (): Promise<void> => {
    if (!isTauri()) {
      console.debug('[app-update] hook.install: skipped — not running in Tauri');
      return;
    }
    if (!stagedRef.current) {
      console.warn('[app-update] hook.install: no staged update — falling back to apply');
      try {
        await applyAppUpdate();
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        console.error('[app-update] hook.install: apply fallback failed', message);
        if (mountedRef.current) {
          setError(message);
          setPhase('error');
        }
      }
      return;
    }
    console.info('[app-update] hook.install: starting');
    setError(null);
    // The Rust side consumes the staged bytes via `slot.take()` before
    // calling `Update::install`, so once we invoke install_app_update the
    // backend no longer has a pending update — keep `stagedRef` in sync so
    // a retry after a transient install failure falls back to the legacy
    // `apply` path (fresh check + download + install) instead of looping
    // on a now-empty Rust state slot.
    stagedRef.current = false;
    try {
      await installAppUpdate();
      console.debug('[app-update] hook.install: returned without restart');
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.error('[app-update] hook.install: failed', message);
      // Defensive — the early clear above already handled this, but if a
      // future change moves the install_app_update call without resetting
      // the ref, this guarantees retries don't reuse a consumed staging.
      stagedRef.current = false;
      if (mountedRef.current) {
        setError(message);
        setPhase('error');
      }
    }
  }, []);

  /**
   * Legacy combined download+install+restart. Prefer the auto-download flow.
   * Restarts the process mid-promise on success.
   */
  const apply = useCallback(async (): Promise<void> => {
    if (!isTauri()) {
      console.debug('[app-update] hook.apply: skipped — not running in Tauri');
      return;
    }
    console.info('[app-update] hook.apply: starting (legacy path)');
    setBytesDownloaded(0);
    setTotalBytes(null);
    setError(null);
    setPhase('checking');
    try {
      await applyAppUpdate();
      console.debug('[app-update] hook.apply: returned without restart');
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.error('[app-update] hook.apply: failed', message);
      if (mountedRef.current) {
        setError(message);
        setPhase('error');
      }
    }
  }, []);

  const reset = useCallback(() => {
    console.debug('[app-update] hook.reset');
    setError(null);
    setBytesDownloaded(0);
    setTotalBytes(null);
    if (
      phaseRef.current === 'error' ||
      phaseRef.current === 'up_to_date' ||
      phaseRef.current === 'available'
    ) {
      setPhase('idle');
    }
  }, []);

  // Subscribe to Rust-side updater events for the lifetime of the hook.
  useEffect(() => {
    if (!isTauri()) return;

    mountedRef.current = true;
    let unlistenStatus: UnlistenFn | undefined;
    let unlistenProgress: UnlistenFn | undefined;
    let cancelled = false;

    (async () => {
      try {
        unlistenStatus = await listen<string>('app-update:status', event => {
          if (!mountedRef.current) return;
          const next = parseStatusPayload(event.payload);
          console.debug('[app-update] hook: status →', next);
          setPhase(next);
          if (next === 'downloading') {
            setBytesDownloaded(0);
            setTotalBytes(null);
            setError(null);
          }
          if (next === 'ready_to_install') {
            stagedRef.current = true;
          }
          if (next === 'error') {
            setError(prev => prev ?? 'Update failed. See logs for details.');
          }
        });

        unlistenProgress = await listen<AppUpdateProgress>('app-update:progress', event => {
          if (!mountedRef.current) return;
          const { chunk, total } = event.payload ?? { chunk: 0, total: null };
          setBytesDownloaded(prev => prev + (typeof chunk === 'number' ? chunk : 0));
          if (typeof total === 'number') setTotalBytes(total);
        });

        if (cancelled) {
          unlistenStatus?.();
          unlistenProgress?.();
        }
      } catch (err) {
        console.debug('[app-update] hook: failed to attach listeners', err);
      }
    })();

    return () => {
      cancelled = true;
      mountedRef.current = false;
      unlistenStatus?.();
      unlistenProgress?.();
    };
  }, []);

  // Auto-check cadence: one delayed probe, then a periodic re-probe.
  useEffect(() => {
    if (!autoCheck || !isTauri()) return;

    const initialTimer = setTimeout(
      () => {
        void check();
      },
      Math.max(0, initialCheckDelayMs)
    );

    let recheckTimer: ReturnType<typeof setInterval> | undefined;
    if (recheckIntervalMs > 0) {
      recheckTimer = setInterval(() => {
        void check();
      }, recheckIntervalMs);
    }

    return () => {
      clearTimeout(initialTimer);
      if (recheckTimer) clearInterval(recheckTimer);
    };
  }, [autoCheck, initialCheckDelayMs, recheckIntervalMs, check]);

  // Auto-download: when a check transitions us to `available`, kick off a
  // background download so the user is only ever asked to restart, never to
  // download.
  useEffect(() => {
    if (!autoDownload || !isTauri()) return;
    if (phase !== 'available') return;
    if (downloadInFlightRef.current) return;

    const timer = setTimeout(() => {
      void download();
    }, AUTO_DOWNLOAD_GRACE_MS);

    return () => clearTimeout(timer);
  }, [autoDownload, phase, download]);

  return {
    phase,
    info,
    bytesDownloaded,
    totalBytes,
    error,
    check,
    download,
    install,
    apply,
    reset,
  };
}
