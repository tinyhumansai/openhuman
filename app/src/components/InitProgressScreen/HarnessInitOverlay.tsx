import debugFactory from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import {
  fetchHarnessInitStatus,
  type HarnessInitSnapshot,
  runHarnessInit,
} from '../../services/harnessInitService';
import InitProgressScreen from './InitProgressScreen';

const log = debugFactory('harness-init');

const POLL_MS = 2000;

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

  useEffect(() => {
    cancelledRef.current = false;
    let timeoutId: number | null = null;

    const poll = async () => {
      try {
        const next = await fetchHarnessInitStatus();
        if (cancelledRef.current || dismissedRef.current) {
          return;
        }
        if (next) {
          setSnapshot(next);
          // Stop polling once the run is terminal; a `failed` snapshot stays
          // on screen (with Retry) but does not need further polling.
          if (next.overall === 'done' || next.overall === 'failed') {
            return;
          }
        }
      } catch (err) {
        // Status can fail while the core is still coming up — keep polling.
        log('status poll failed: %O', err);
      }
      if (!cancelledRef.current && !dismissedRef.current) {
        timeoutId = window.setTimeout(() => void poll(), POLL_MS);
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
    // background task regardless.
    dismissedRef.current = true;
    setDismissed(true);
  }, []);

  if (dismissed || !snapshot) {
    return null;
  }

  // Block only while a run is actively in progress, or hold a failed run on
  // screen until the user explicitly continues. `idle` (no run started yet)
  // and `done` never block.
  const shouldShow = snapshot.overall === 'running' || snapshot.overall === 'failed';
  if (!shouldShow) {
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
