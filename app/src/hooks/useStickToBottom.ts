import { useEffect, useLayoutEffect, useRef } from 'react';

/**
 * Keep a scroll container pinned to the bottom as messages arrive.
 *
 * Three observers cooperate:
 * 1. Layout-effect on `messages` / `threadKey` / `resetKey` — handles thread
 *    swaps and the first paint, instantly snapping to the latest message.
 * 2. `scroll` listener — toggles `stickingRef` based on the user's distance
 *    from the bottom so manual scroll-up disengages the auto-snap.
 * 3. ResizeObserver on the container *and its children*, plus a
 *    MutationObserver on the container's `childList` that re-binds the
 *    ResizeObserver whenever the subtree is swapped. This keeps streaming
 *    agent replies in view: each token chunk grows the content height,
 *    the resize observer fires, and we snap to the new bottom before paint.
 *
 * If the user manually scrolls up past the threshold we stop sticking, so they
 * can read history without being yanked down. Scrolling back to the bottom
 * re-engages stickiness on the next render.
 */

const STICK_THRESHOLD_PX = 80;

function isNearBottom(el: HTMLElement): boolean {
  return el.scrollHeight - el.scrollTop - el.clientHeight <= STICK_THRESHOLD_PX;
}

function snapToBottom(el: HTMLElement) {
  el.scrollTop = el.scrollHeight;
}

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
  // Tracks whether we should keep auto-scrolling. Flips to false when the user
  // scrolls up away from the bottom; flips back when they return.
  const stickingRef = useRef(true);

  // ── Snap on message / thread / route changes ─────────────────────────────
  useLayoutEffect(() => {
    if (lastResetKeyRef.current !== resetKey) {
      didInitialScrollRef.current = false;
      lastResetKeyRef.current = resetKey;
    }
    // Record the active thread on every render (including empty ones) so
    // the A → empty B → A navigation pattern is recognised as a thread
    // change when A's messages re-arrive.
    const previousThread = lastScrolledThreadRef.current;
    lastScrolledThreadRef.current = threadKey ?? null;
    if (messages.length === 0) return;
    const container = containerRef.current;
    if (!container) return;

    const threadChanged = previousThread !== threadKey;
    const firstScroll = !didInitialScrollRef.current;
    if (firstScroll || threadChanged || stickingRef.current) {
      snapToBottom(container);
      stickingRef.current = true;
    }
    didInitialScrollRef.current = true;
  }, [messages, threadKey, resetKey]);

  // ── Track manual scroll → toggle stickingRef ─────────────────────────────
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const onScroll = () => {
      stickingRef.current = isNearBottom(container);
    };
    container.addEventListener('scroll', onScroll, { passive: true });
    return () => container.removeEventListener('scroll', onScroll);
  }, []);

  // ── Pin to bottom while content grows (streaming chunks) ─────────────────
  //
  // The ResizeObserver only fires for elements it's currently observing, so
  // when the container's subtree gets swapped (e.g. switching from the
  // welcome loader to the message list, or from one thread to another),
  // we have to re-observe the new children. A MutationObserver on
  // `childList` does that automatically.
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const resizeObserver = new ResizeObserver(() => {
      if (stickingRef.current) {
        snapToBottom(container);
      }
    });

    const observeAllChildren = () => {
      // Disconnect first so we don't end up holding stale child refs after
      // a subtree swap; then re-attach to the container and every direct
      // child currently mounted.
      resizeObserver.disconnect();
      resizeObserver.observe(container);
      for (let child = container.firstElementChild; child; child = child.nextElementSibling) {
        resizeObserver.observe(child);
      }
    };

    observeAllChildren();

    const mutationObserver = new MutationObserver(() => observeAllChildren());
    mutationObserver.observe(container, { childList: true });

    return () => {
      resizeObserver.disconnect();
      mutationObserver.disconnect();
    };
  }, []);

  return { containerRef, endRef };
}
