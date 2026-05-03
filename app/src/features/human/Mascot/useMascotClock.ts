import { useEffect, useState } from 'react';

/**
 * RAF-driven elapsed-time clock in seconds since mount. Replaces Remotion's
 * useCurrentFrame for runtime rendering.
 */
export function useMascotClock(active = true): number {
  const [seconds, setSeconds] = useState(0);

  useEffect(() => {
    if (!active) return;
    let raf = 0;
    const start = performance.now();
    const tick = (now: number) => {
      setSeconds((now - start) / 1000);
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [active]);

  return seconds;
}
