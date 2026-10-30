import { useEffect, useRef, useState } from 'react';

import { useAnimationActivity } from '../../../hooks/useAnimationActivity';

/**
 * RAF-driven active time in seconds. Hidden/reduced-motion views keep their
 * last frame and resume from it instead of jumping or restarting the clock.
 */
export function useMascotClock(active = true): number {
  const [seconds, setSeconds] = useState(0);
  const elapsed = useRef(0);
  const animationAllowed = useAnimationActivity();

  useEffect(() => {
    if (!active || !animationAllowed) return;
    let raf = 0;
    const start = window.performance.now();
    const previous = elapsed.current;
    const tick = (now: number) => {
      elapsed.current = previous + (now - start) / 1000;
      setSeconds(elapsed.current);
      raf = window.requestAnimationFrame(tick);
    };
    raf = window.requestAnimationFrame(tick);
    return () => window.cancelAnimationFrame(raf);
  }, [active, animationAllowed]);

  return seconds;
}
