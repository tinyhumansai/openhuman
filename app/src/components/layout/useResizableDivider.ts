import { useCallback, useLayoutEffect, useRef, useState } from 'react';

/**
 * Pointer-drag + keyboard resize mechanics for a single divider, with the
 * "commit on release" persistence contract `TwoPanelLayout` needs: live drag
 * width is kept in local state (applied via inline style) and only handed to
 * `onCommit` once, on pointer-up or a keyboard step — never on every
 * pointermove frame — so a Redux-persisted store isn't thrashed mid-drag.
 *
 * This is deliberately NOT shared code with `ui/Sidebar.tsx`'s `SidebarRail`:
 * that primitive's `onWidthChange` fires on every drag frame by contract (its
 * own doc comment says so) because the shell mirrors sidebar width into
 * `--sidebar-width` for live layout reflow. `TwoPanelLayout`'s panes don't
 * need that — they resize from local `dragWidth` state — and persisting on
 * every frame is exactly what this hook exists to avoid. Pulled out of the
 * component body into its own hook so the clamp/drag/keyboard logic is one
 * named, independently readable unit rather than duplicated inline should a
 * second two-pane splitter appear in this directory.
 */
export function clampWidth(width: number, min: number, max: number): number {
  return Math.min(Math.max(width, min), max);
}

interface UseResizableDividerOptions {
  /** Current persisted width, used as the drag/keyboard starting point. */
  width: number;
  minWidth: number;
  maxWidth: number;
  keyboardStep: number;
  /** Called once per drag (on release) or once per arrow-key press, with the final clamped width. */
  onCommit: (width: number) => void;
}

interface UseResizableDividerResult {
  /** Live width while dragging (`null` when not dragging) — apply via inline style, falling back to the persisted width. */
  dragWidth: number | null;
  onPointerDown: (e: React.PointerEvent) => void;
  onKeyDown: (e: React.KeyboardEvent) => void;
}

export function useResizableDivider({
  width,
  minWidth,
  maxWidth,
  keyboardStep,
  onCommit,
}: UseResizableDividerOptions): UseResizableDividerResult {
  const [dragWidth, setDragWidth] = useState<number | null>(null);
  const dragWidthRef = useRef<number | null>(null);

  const commitWidth = useCallback(
    (next: number) => {
      onCommit(clampWidth(Math.round(next), minWidth, maxWidth));
    },
    [onCommit, minWidth, maxWidth]
  );

  // Active-drag teardown, stashed so an unmount mid-drag can detach the global
  // listeners. Each drag installs locally-scoped `pointermove`/`pointerup`
  // handlers (hoisted function declarations so they can reference each other),
  // keeping the resize self-contained without inter-callback dependencies.
  const dragCleanupRef = useRef<(() => void) | null>(null);

  const onPointerDown = useCallback(
    (e: React.PointerEvent) => {
      e.preventDefault();
      const startX = e.clientX;
      const startWidth = width;
      dragWidthRef.current = startWidth;
      setDragWidth(startWidth);
      document.body.style.cursor = 'col-resize';
      document.body.style.userSelect = 'none';

      function handleMove(ev: PointerEvent) {
        const next = clampWidth(startWidth + (ev.clientX - startX), minWidth, maxWidth);
        dragWidthRef.current = next;
        setDragWidth(next);
      }
      function detach() {
        window.removeEventListener('pointermove', handleMove);
        window.removeEventListener('pointerup', stop);
        document.body.style.removeProperty('cursor');
        document.body.style.removeProperty('user-select');
        dragCleanupRef.current = null;
      }
      function stop() {
        detach();
        const finalWidth = dragWidthRef.current;
        dragWidthRef.current = null;
        setDragWidth(null);
        if (finalWidth != null) commitWidth(finalWidth);
      }

      dragCleanupRef.current = detach;
      window.addEventListener('pointermove', handleMove);
      window.addEventListener('pointerup', stop);
    },
    [width, minWidth, maxWidth, commitWidth]
  );

  // Detach global listeners if we unmount mid-drag.
  useLayoutEffect(() => {
    return () => {
      dragCleanupRef.current?.();
    };
  }, []);

  const onKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'ArrowLeft') {
        e.preventDefault();
        commitWidth(width - keyboardStep);
      } else if (e.key === 'ArrowRight') {
        e.preventDefault();
        commitWidth(width + keyboardStep);
      }
    },
    [commitWidth, width, keyboardStep]
  );

  return { dragWidth, onPointerDown, onKeyDown };
}
