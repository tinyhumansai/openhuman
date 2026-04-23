import { useLayoutEffect, useRef } from 'react';

export function useStickToBottom(
  messages: readonly unknown[],
  threadKey: string | null | undefined,
  resetKey: string
) {
  const containerRef = useRef<HTMLDivElement>(null);
  const endRef = useRef<HTMLDivElement>(null);
  const didInitialScrollRef = useRef(false);
  const lastScrolledThreadRef = useRef<string | null>(null);
  const lastResetKeyRef = useRef(resetKey);

  useLayoutEffect(() => {
    // Reset is handled inside the same layout phase as the scroll so we never
    // read a stale `didInitialScrollRef` after a resetKey change (the previous
    // useEffect-based reset fired after paint, leaving this layout effect with
    // stale state on the first re-render and triggering an unwanted smooth
    // scroll animation on re-entry).
    if (lastResetKeyRef.current !== resetKey) {
      didInitialScrollRef.current = false;
      lastResetKeyRef.current = resetKey;
    }
    if (messages.length === 0) return;
    const container = containerRef.current;
    const threadChanged = lastScrolledThreadRef.current !== threadKey;
    const firstScroll = !didInitialScrollRef.current;
    const instant = firstScroll || threadChanged;
    if (instant) {
      if (container) {
        container.scrollTop = container.scrollHeight;
      }
    } else {
      endRef.current?.scrollIntoView({ behavior: 'smooth', block: 'end' });
    }
    lastScrolledThreadRef.current = threadKey ?? null;
    didInitialScrollRef.current = true;
  }, [messages, threadKey, resetKey]);

  return { containerRef, endRef };
}
