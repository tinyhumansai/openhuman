import { useSyncExternalStore } from 'react';

const MOTION_QUERY = '(prefers-reduced-motion: reduce)';

function animationAllowed(): boolean {
  return !document.hidden && !window.matchMedia?.(MOTION_QUERY).matches;
}

function subscribe(onChange: () => void): () => void {
  const media = window.matchMedia?.(MOTION_QUERY);
  document.addEventListener('visibilitychange', onChange);
  if (media?.addEventListener) media.addEventListener('change', onChange);
  else media?.addListener(onChange);
  return () => {
    document.removeEventListener('visibilitychange', onChange);
    if (media?.removeEventListener) media.removeEventListener('change', onChange);
    else media?.removeListener(onChange);
  };
}

/** Decorative clocks and idle flourishes only run while the page is visible. */
export function useAnimationActivity(): boolean {
  return useSyncExternalStore(subscribe, animationAllowed, () => false);
}
