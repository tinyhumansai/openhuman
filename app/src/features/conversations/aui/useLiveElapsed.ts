import { useEffect, useState } from 'react';

/**
 * Milliseconds since `since` (epoch ms), re-rendering once a second while
 * `active`. Returns `undefined` when inactive or when `since` is unknown, so a
 * caller never shows a frozen clock.
 */
export function useLiveElapsed(since: number | undefined, active: boolean): number | undefined {
  const ticking = active && since !== undefined;
  const [now, setNow] = useState(() => Date.now());
  const [wasTicking, setWasTicking] = useState(ticking);
  if (wasTicking !== ticking) {
    setWasTicking(ticking);
    if (ticking) setNow(Date.now());
  }
  useEffect(() => {
    if (!ticking) return undefined;
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [ticking]);
  if (!ticking || since === undefined) return undefined;
  return Math.max(0, now - since);
}

/**
 * Epoch ms at which `running` last became true (or the component mounted
 * running), cleared when it stops. For live timers on rows that carry no
 * start timestamp of their own.
 */
export function useRunningSince(running: boolean): number | undefined {
  const [since, setSince] = useState<number | undefined>(() => (running ? Date.now() : undefined));
  const [wasRunning, setWasRunning] = useState(running);
  if (wasRunning !== running) {
    setWasRunning(running);
    setSince(running ? Date.now() : undefined);
  }
  return since;
}
