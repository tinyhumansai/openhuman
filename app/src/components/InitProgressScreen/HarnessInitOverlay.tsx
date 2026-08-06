import debugFactory from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { isMethodNotFoundCoreRpcError } from '../../services/coreRpcClient';
import {
  fetchHarnessInitStatus,
  type HarnessInitSnapshot,
  runHarnessInit,
} from '../../services/harnessInitService';
import InitProgressScreen from './InitProgressScreen';

const log = debugFactory('harness-init');

const POLL_MS = 2000;

// A status poll can legitimately fail while the core is still coming up, so a
// failure is retried. But the retry must be *bounded*: before #5157 any failure
// rescheduled the poll unconditionally every 2s for the life of the window. A
// core that never serves this method (version skew / domain-gated build) turned
// that into a permanent 30-calls-per-minute loop, and because each miss is
// recorded core-side it produced ~9k Sentry events/day from a single client.
//
// Two guards now bound it:
//   - a permanent failure (`method_not_found`) stops the loop immediately —
//     retrying an absent method can never succeed;
//   - any other failure gets a capped number of attempts with exponential
//     backoff, so even an unforeseen persistent fault decays and stops.
const MAX_TRANSIENT_FAILURES = 5;
const MAX_BACKOFF_MS = 30_000;

// Exhausting the retries must not strand a *blocking* overlay on stale
// progress. While a `running` snapshot is on screen the UI is covering the app
// and waiting for a terminal snapshot, so a transient outage (brief core
// overload, network blip) that outlasts the cap has to stay watchable — giving
// up there would freeze the overlay on a half-finished run until the user hits
// "Run in background", and the pre-#5157 loop did recover from exactly that.
//
// So the cap still applies whenever nothing blocking is displayed (the #5157
// case: a core that never serves this method, with no UI on screen), and
// otherwise the loop drops to this much slower cadence instead of stopping.
// 2 calls/min is 15x below the runaway loop #5157 fixed, and only ever runs
// while a blocking overlay is actually up.
const STALLED_POLL_MS = MAX_BACKOFF_MS;

/** Backoff for the Nth consecutive transient failure (1-based), capped. */
function transientRetryDelayMs(consecutiveFailures: number): number {
  return Math.min(POLL_MS * 2 ** (consecutiveFailures - 1), MAX_BACKOFF_MS);
}

/**
 * Whether this snapshot puts the blocking overlay on screen. `running` is also
 * the only non-terminal blocking state, so it is what the poll loop must keep
 * watching for a terminal result.
 */
function isBlockingSnapshot(snapshot: HarnessInitSnapshot): boolean {
  return snapshot.overall === 'running' || snapshot.overall === 'failed';
}

// Persist the "Run in background" dismissal for the *current* provisioning run
// so a remount or reload does not reopen the overlay (GH-5047). A run is keyed
// by its `startedAt` timestamp — a genuinely new provisioning run gets a fresh
// timestamp and is allowed to surface again. `sessionStorage` survives a
// renderer reload within the same window; a module-level mirror covers plain
// React remounts even if storage is unavailable.
const DISMISS_KEY = 'harness-init-dismissed-run';
// Runs before `startedAt` is stamped (or when it is absent) still need a stable
// key so an early dismissal sticks.
const UNKEYED_RUN = 'pending';

let dismissedRunMirror: string | null = null;

// Coalesce overlapping status polls onto a single in-flight request. React
// StrictMode double-mounts this overlay in dev (effect → cleanup → effect),
// and each setup fires an immediate poll — without this that boots two
// `harness_init_status` RPCs at the same instant. Concurrent callers share the
// in-flight promise; it clears once settled, so the ongoing (sequential) poll
// loop is unaffected. Also guards any genuine remount during the boot window.
let inflightStatusFetch: Promise<HarnessInitSnapshot | null> | null = null;

function fetchHarnessInitStatusCoalesced(): Promise<HarnessInitSnapshot | null> {
  if (inflightStatusFetch) {
    log('status poll: joining in-flight request (coalesced)');
    return inflightStatusFetch;
  }
  log('status poll: dispatching harness_init_status');
  const pending = fetchHarnessInitStatus().finally(() => {
    if (inflightStatusFetch === pending) {
      inflightStatusFetch = null;
      log('status poll: in-flight request settled, cache cleared');
    }
  });
  inflightStatusFetch = pending;
  return pending;
}

function runKey(snapshot: HarnessInitSnapshot | null): string {
  return snapshot?.startedAt ?? UNKEYED_RUN;
}

function readDismissedRun(): string | null {
  if (dismissedRunMirror !== null) {
    return dismissedRunMirror;
  }
  try {
    return window.sessionStorage.getItem(DISMISS_KEY);
  } catch {
    log('sessionStorage read failed; treating run as not dismissed');
    return null;
  }
}

function writeDismissedRun(key: string): void {
  dismissedRunMirror = key;
  try {
    window.sessionStorage.setItem(DISMISS_KEY, key);
    log('dismissed run persisted to sessionStorage: %s', key);
  } catch {
    // Non-fatal: the module-level mirror still guards remounts this session.
    log('sessionStorage unavailable; dismissed run %s held in module mirror only', key);
  }
}

function isRunDismissed(snapshot: HarnessInitSnapshot | null): boolean {
  return readDismissedRun() === runKey(snapshot);
}

/**
 * Blocking first-run initialization gate.
 *
 * Polls `openhuman.harness_init_status` and, while the run is in progress,
 * covers the app with a full-screen overlay showing per-step progress. The
 * overlay offers a "Run in background" action so the user can dismiss it and
 * keep working while setup continues — the core runs init as a background task
 * regardless of whether the overlay is shown. On a warm host every step is
 * already provisioned, so the snapshot reports `done` on the first poll and
 * this renders nothing. On a terminal `failed` it offers Retry / Continue —
 * failures are non-fatal (the core degrades to a fallback).
 *
 * Polling-based (not socket) to sidestep the cold-start race where the socket
 * is not yet connected when init begins.
 */
export default function HarnessInitOverlay() {
  const [snapshot, setSnapshot] = useState<HarnessInitSnapshot | null>(null);
  const [dismissed, setDismissed] = useState(false);
  const [retrying, setRetrying] = useState(false);
  const cancelledRef = useRef(false);
  // Mirrors `dismissed` so the poll loop can stop without re-running the effect.
  const dismissedRef = useRef(false);
  // Mirrors "a blocking overlay is currently on screen, still waiting on a
  // terminal snapshot", so the failure branch can tell a stranded user from a
  // silent background loop without re-running the effect.
  const awaitingTerminalRef = useRef(false);

  useEffect(() => {
    cancelledRef.current = false;
    let timeoutId: number | null = null;

    let consecutiveFailures = 0;

    const poll = async () => {
      let retryDelayMs = POLL_MS;
      try {
        const next = await fetchHarnessInitStatusCoalesced();
        if (cancelledRef.current || dismissedRef.current) {
          return;
        }
        consecutiveFailures = 0;
        if (next) {
          setSnapshot(next);
          awaitingTerminalRef.current = next.overall === 'running';
          // If this run was already dismissed to the background (possibly in a
          // prior mount / before a reload), stay hidden and stop polling —
          // don't let a remount reopen the overlay (GH-5047).
          if (isRunDismissed(next)) {
            log(
              'warm poll: run %s already dismissed — staying hidden, stopping poll',
              runKey(next)
            );
            dismissedRef.current = true;
            setDismissed(true);
            return;
          }
          // Stop polling once the run is terminal; a `failed` snapshot stays
          // on screen (with Retry) but does not need further polling.
          if (next.overall === 'done' || next.overall === 'failed') {
            return;
          }
        }
      } catch (err) {
        if (cancelledRef.current || dismissedRef.current) {
          return;
        }
        // The running core has no `harness_init_status` at all (version skew,
        // domain-gated or slim build). Permanent — stop, and render nothing.
        // There is no init run to report, so there is nothing to show (#5157).
        if (isMethodNotFoundCoreRpcError(err)) {
          log('status poll: core does not expose harness_init_status — stopping poll');
          return;
        }
        consecutiveFailures += 1;
        // Status can fail while the core is still coming up — keep polling, but
        // only for a bounded number of attempts, backing off between each.
        if (consecutiveFailures >= MAX_TRANSIENT_FAILURES) {
          // Nothing blocking on screen: this is the #5157 runaway loop, stop.
          if (!awaitingTerminalRef.current) {
            log(
              'status poll failed %d consecutive times — giving up: %O',
              consecutiveFailures,
              err
            );
            return;
          }
          // A `running` overlay is covering the app. Stopping here would pin it
          // to stale progress for the rest of the session even after the core
          // recovers, so keep watching at a much slower cadence instead.
          retryDelayMs = STALLED_POLL_MS;
          log(
            'status poll failed %d consecutive times but a running overlay is on screen — ' +
              'continuing at %dms: %O',
            consecutiveFailures,
            retryDelayMs,
            err
          );
        } else {
          retryDelayMs = transientRetryDelayMs(consecutiveFailures);
          log(
            'status poll failed (attempt %d/%d), retrying in %dms: %O',
            consecutiveFailures,
            MAX_TRANSIENT_FAILURES,
            retryDelayMs,
            err
          );
        }
      }
      if (!cancelledRef.current && !dismissedRef.current) {
        timeoutId = window.setTimeout(() => void poll(), retryDelayMs);
      }
    };

    void poll();

    return () => {
      cancelledRef.current = true;
      if (timeoutId !== null) {
        window.clearTimeout(timeoutId);
      }
    };
  }, []);

  const handleRetry = useCallback(async () => {
    setRetrying(true);
    try {
      const next = await runHarnessInit(false);
      if (next) {
        setSnapshot(next);
      }
    } catch (err) {
      log('retry failed: %O', err);
    } finally {
      setRetrying(false);
    }
  }, []);

  const handleContinue = useCallback(() => {
    // Hide the overlay and stop polling; the core keeps running init as a
    // background task regardless. Persist the dismissal for this run so a
    // remount/reload does not reopen it (GH-5047).
    log('user dismissed overlay to background for run %s', runKey(snapshot));
    writeDismissedRun(runKey(snapshot));
    dismissedRef.current = true;
    // Nothing is blocking any more, so a later failure must not keep the slow
    // watch alive. (`dismissedRef` already stops the loop; this keeps the two
    // flags from disagreeing.)
    awaitingTerminalRef.current = false;
    setDismissed(true);
  }, [snapshot]);

  if (dismissed || !snapshot) {
    return null;
  }

  // A run dismissed to the background stays hidden across remounts.
  if (isRunDismissed(snapshot)) {
    return null;
  }

  // Block only while a run is actively in progress, or hold a failed run on
  // screen until the user explicitly continues. `idle` (no run started yet)
  // and `done` never block.
  if (!isBlockingSnapshot(snapshot)) {
    return null;
  }

  return (
    <InitProgressScreen
      snapshot={snapshot}
      onRetry={handleRetry}
      onContinue={handleContinue}
      retrying={retrying}
    />
  );
}
