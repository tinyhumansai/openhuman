import { type ReactNode, useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import {
  ensurePanelLayout,
  selectPanelLayout,
  setSidebarVisible,
  setSidebarWidth,
  toggleSidebar,
} from '../../../store/layoutSlice';
import { Tooltip } from '../../ui';
import CollapsedNavRail from './CollapsedNavRail';
import WindowDragBar from './WindowDragBar';

// `app-shell` (not the older `root-shell`) so the persisted geometry seeds
// fresh with the sidebar visible by default. Exported so the global command
// layer (mod+B "Toggle sidebar") can target this exact panel.
export const APP_SHELL_LAYOUT_ID = 'app-shell';
const LAYOUT_ID = APP_SHELL_LAYOUT_ID;
const DEFAULT_WIDTH = 224;
const MIN_WIDTH = 188;
const MAX_WIDTH = 420;
const KEYBOARD_STEP = 16;
const LAYOUT_DEFAULTS = { sidebarVisible: true, sidebarWidth: DEFAULT_WIDTH };

function clamp(width: number): number {
  return Math.min(Math.max(width, MIN_WIDTH), MAX_WIDTH);
}

/**
 * Subscribe to the root shell sidebar's visibility and get helpers to drive it
 * from chrome that lives elsewhere (e.g. the in-sidebar header's collapse
 * button, or a reshow button in the content area).
 */
export function useRootSidebar() {
  const dispatch = useAppDispatch();
  const layout = useAppSelector(selectPanelLayout(LAYOUT_ID, LAYOUT_DEFAULTS));
  return {
    visible: layout.sidebarVisible,
    toggle: useCallback(() => dispatch(toggleSidebar({ id: LAYOUT_ID })), [dispatch]),
    show: useCallback(
      () => dispatch(setSidebarVisible({ id: LAYOUT_ID, visible: true })),
      [dispatch]
    ),
    hide: useCallback(
      () => dispatch(setSidebarVisible({ id: LAYOUT_ID, visible: false })),
      [dispatch]
    ),
  };
}

export interface RootShellLayoutProps {
  /** Always-visible left pane (the app sidebar). */
  sidebar: ReactNode;
  /** Dynamic main content (the routed page area). */
  children: ReactNode;
}

/**
 * Full-bleed, viewport-filling two-pane shell for the app root: a resizable
 * sidebar on the left and the main content on the right, separated by a flush
 * hairline seam. Unlike the in-page {@link TwoPanelLayout}, this fills its
 * container edge-to-edge (no card, no rounded corners) because it *is* the
 * window chrome. The dragged width persists per user via the `layout` slice
 * (id `root-shell`); the sidebar is always shown.
 */
export default function RootShellLayout({ sidebar, children }: RootShellLayoutProps) {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const layout = useAppSelector(selectPanelLayout(LAYOUT_ID, LAYOUT_DEFAULTS));
  const persistedWidth = clamp(layout.sidebarWidth);
  const isOpen = layout.sidebarVisible;

  // Seed persisted geometry once so the selector returns a stable stored
  // reference on subsequent renders (avoids the new-object memoization warning).
  useEffect(() => {
    dispatch(ensurePanelLayout({ id: LAYOUT_ID, defaults: LAYOUT_DEFAULTS }));
  }, [dispatch]);

  const [dragWidth, setDragWidth] = useState<number | null>(null);
  const dragWidthRef = useRef<number | null>(null);
  const dragCleanupRef = useRef<(() => void) | null>(null);
  const width = dragWidth ?? persistedWidth;

  const commitWidth = useCallback(
    (next: number) => dispatch(setSidebarWidth({ id: LAYOUT_ID, width: clamp(Math.round(next)) })),
    [dispatch]
  );

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
        const next = clamp(startWidth + (ev.clientX - startX));
        dragWidthRef.current = next;
        setDragWidth(next);
      }
      function detach() {
        window.removeEventListener('pointermove', handleMove);
        window.removeEventListener('pointerup', stop);
        window.removeEventListener('pointercancel', stop);
        window.removeEventListener('blur', stop);
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
      window.addEventListener('pointercancel', stop);
      window.addEventListener('blur', stop);
    },
    [width, commitWidth]
  );

  // Detach global listeners if we unmount mid-drag.
  useLayoutEffect(() => () => dragCleanupRef.current?.(), []);

  const onDividerKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'ArrowLeft') {
        e.preventDefault();
        commitWidth(persistedWidth - KEYBOARD_STEP);
      } else if (e.key === 'ArrowRight') {
        e.preventDefault();
        commitWidth(persistedWidth + KEYBOARD_STEP);
      }
    },
    [commitWidth, persistedWidth]
  );

  return (
    <div className="relative flex h-full w-full min-h-0 overflow-hidden">
      {isOpen && (
        <>
          <div
            className="flex-shrink-0 min-w-0 overflow-hidden"
            style={{ width }}
            data-testid="root-shell-sidebar">
            {sidebar}
          </div>

          <div
            role="separator"
            aria-orientation="vertical"
            aria-label={t('layout.resizeSidebar')}
            aria-valuenow={Math.round(width)}
            aria-valuemin={MIN_WIDTH}
            aria-valuemax={MAX_WIDTH}
            tabIndex={0}
            data-testid="root-shell-divider"
            data-analytics-id="root-shell-resize-divider"
            onPointerDown={onPointerDown}
            onKeyDown={onDividerKeyDown}
            title={t('layout.resizeSidebar')}
            className="group relative w-px flex-shrink-0 cursor-col-resize select-none self-stretch bg-surface-strong focus:outline-none">
            <span className="absolute inset-y-0 -left-1 -right-1 z-10" />
            <span className="absolute inset-0 transition-colors group-hover:bg-primary-400 group-focus:bg-primary-500" />
          </div>
        </>
      )}

      {/* Reshow affordance — only when the sidebar is collapsed. A thin rail
          that occupies layout space (NOT an overlay) so the content — and the
          native CEF webview glued to the content's bounds, which composites
          above the HTML layer — starts to its right and never covers it. */}
      {!isOpen && (
        <div className="flex w-14 flex-none flex-col items-center gap-0.5 border-r border-line bg-surface">
          {/* macOS overlay title bar (titleBarStyle: Overlay) floats the traffic
              lights over the top-left. The expanded SidebarHeader dodges them by
              right-aligning, but this narrow rail can't — so reserve a draggable
              strip the height of the window controls and start the rail below it,
              clear of the lights. */}
          <div className="h-7 w-full flex-none" data-tauri-drag-region />
          <Tooltip label={t('layout.showSidebar')}>
            <button
              type="button"
              onClick={() => dispatch(setSidebarVisible({ id: LAYOUT_ID, visible: true }))}
              data-testid="root-shell-reopen"
              data-analytics-id="root-shell-reopen-sidebar"
              aria-label={t('layout.showSidebar')}
              className="flex h-8 w-8 items-center justify-center rounded-lg text-content-muted transition-colors hover:bg-surface-hover hover:text-content-secondary">
              <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={1.8}
                  d="M9 5l7 7-7 7"
                />
              </svg>
            </button>
          </Tooltip>
          {/* Keep the primary nav reachable while collapsed: an icon-only rail. */}
          <div className="mt-1 w-full border-t border-line/70 pt-1 dark:border-line/70">
            <CollapsedNavRail />
          </div>
        </div>
      )}

      <div className="relative flex-1 min-w-0 overflow-hidden" data-testid="root-shell-content">
        {children}
        {/* macOS overlay-title-bar drag strip — a transparent overlay pinned on
            TOP of the routed view (last child) so full-bleed surfaces (Tiny
            Place world, Chat backdrop) stay edge-to-edge while the top of the
            window still drags. The sidebar is excluded — its header already
            drags in place. No-op off macOS / outside Tauri. */}
        <WindowDragBar />
      </div>
    </div>
  );
}
