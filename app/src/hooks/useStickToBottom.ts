import createDebug from 'debug';
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
 *
 * Root-cause note (copilot "chat gets stuck" bug): a naive `useEffect` that
 * unconditionally calls `scrollTo(bottom)` on every dependency change (new
 * message, streamed token, tool-timeline entry, …) fights a user who has
 * scrolled up to read — it snaps them back down mid-read, which reads as the
 * scroll container "locking up". This hook is the fix for that pattern:
 * auto-scroll only fires while `stickingRef` is true (i.e. the user was
 * already at/near the bottom), so scrolling up always disengages it.
 */

const log = createDebug('app:hooks:stick-to-bottom');

const STICK_THRESHOLD_PX = 80;

/** Pure, independently-testable "is this container at/near the bottom?" check. */
export function isNearBottom(el: HTMLElement, thresholdPx: number = STICK_THRESHOLD_PX): boolean {
  return el.scrollHeight - el.scrollTop - el.clientHeight <= thresholdPx;
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
  // Mirrors `threadKey` for the mount-only resize-observer effect's log lines
  // below — kept current every render so the logs stay accurate even though
  // that effect itself intentionally doesn't re-run on a thread change (the
  // MutationObserver already re-binds the ResizeObserver on subtree swaps).
  const threadKeyRef = useRef(threadKey);
  threadKeyRef.current = threadKey;

  // ── Snap on message / thread / route changes ─────────────────────────────
  useLayoutEffect(() => {
    if (lastResetKeyRef.current !== resetKey) {
      didInitialScrollRef.current = false;
      lastResetKeyRef.current = resetKey;
    }
    // Record the active thread on every render (including empty ones) so
    // the A → empty B → A navigation pattern is recognised as a thread
    // change when A's messages re-arrive. Normalize `undefined` to `null`
    // on BOTH sides of the comparison below — comparing the normalized
    // previous value against a raw `threadKey` that happens to be
    // `undefined` (rather than `null`) would make `threadChanged` true on
    // every single run (`null !== undefined`), forcing an unwanted snap on
    // every re-render for any caller whose "no thread yet" state is
    // `undefined` instead of `null`. The param type explicitly allows
    // `undefined`, so this normalization has to be symmetric.
    const previousThread = lastScrolledThreadRef.current;
    const normalizedThreadKey = threadKey ?? null;
    lastScrolledThreadRef.current = normalizedThreadKey;
    if (messages.length === 0) return;
    const container = containerRef.current;
    if (!container) return;

    const threadChanged = previousThread !== normalizedThreadKey;
    const firstScroll = !didInitialScrollRef.current;
    if (firstScroll || threadChanged || stickingRef.current) {
      log(
        'layout-effect: snap fired (firstScroll=%s threadChanged=%s sticking=%s) count=%d thread=%s',
        firstScroll,
        threadChanged,
        stickingRef.current,
        messages.length,
        normalizedThreadKey ?? 'null'
      );
      snapToBottom(container);
      stickingRef.current = true;
    } else {
      log(
        'layout-effect: snap skipped (user scrolled up) count=%d thread=%s',
        messages.length,
        normalizedThreadKey ?? 'null'
      );
    }
    didInitialScrollRef.current = true;
  }, [messages, threadKey, resetKey]);

  // ── Track manual scroll → toggle stickingRef ─────────────────────────────
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const onScroll = () => {
      const atBottom = isNearBottom(container);
      if (atBottom !== stickingRef.current) {
        log('scroll: sticking %s -> %s (isNearBottom=%s)', stickingRef.current, atBottom, atBottom);
      }
      stickingRef.current = atBottom;
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
        log('resize-observer: snap fired (sticking) thread=%s', threadKeyRef.current ?? 'null');
        snapToBottom(container);
      } else {
        log(
          'resize-observer: snap skipped (user scrolled up) thread=%s',
          threadKeyRef.current ?? 'null'
        );
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
