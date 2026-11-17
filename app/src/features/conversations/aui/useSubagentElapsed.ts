import { useEffect, useState } from 'react';

/**
 * Elapsed time for a sub-agent delegation that ticks every second while it
 * works, after OpenClaw's live subagent timers.
 *
 * The core reports a delegation's wall-clock `elapsedMs` only once it settles;
 * a running row carries no start time. So the clock is anchored the first time
 * this session sees the task running, in a module-scoped map keyed by task id
 * that outlives assistant-ui's remounts (virtualised history, thread
 * switches). After a reload mid-run the anchor is the reload, so the count
 * restarts — the settled duration from the core replaces it as soon as the
 * run finishes.
 */
const MAX_ANCHORS = 500;
const anchors = new Map<string, number>();

function anchorFor(key: string): number {
  const existing = anchors.get(key);
  if (existing !== undefined) return existing;
  const now = Date.now();
  anchors.set(key, now);
  if (anchors.size > MAX_ANCHORS) {
    const oldest = anchors.keys().next().value;
    if (oldest !== undefined) anchors.delete(oldest);
  }
  return now;
}

/** Forget every anchor. Exposed for tests. */
export function resetSubagentElapsedAnchors(): void {
  anchors.clear();
}

/**
 * `settledMs` once the run has finished, else live milliseconds since the task
 * was first seen running; `undefined` when neither is known.
 */
export function useSubagentElapsed(
  taskId: string | undefined,
  running: boolean,
  settledMs: number | undefined
): number | undefined {
  const ticking = running && taskId !== undefined;
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    if (!ticking) return undefined;
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [ticking]);

  if (!running) return settledMs;
  if (taskId === undefined) return undefined;
  return Math.max(0, now - anchorFor(taskId));
}
