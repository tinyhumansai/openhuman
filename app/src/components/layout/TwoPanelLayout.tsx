import { type ReactNode, useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import {
  ensurePanelLayout,
  type PanelLayout,
  selectPanelLayout,
  setSidebarVisible,
  setSidebarWidth,
  toggleSidebar,
} from '../../store/layoutSlice';

const namespace = 'two-panel-layout';

function debug(message: string, payload?: Record<string, unknown>) {
  if (import.meta.env.DEV) {
    console.debug(`[${namespace}] ${message}`, payload ?? {});
  }
}

function clampWidth(width: number, min: number, max: number): number {
  return Math.min(Math.max(width, min), max);
}

/**
 * Subscribe to a two-pane layout's persisted geometry and get back the
 * helpers external chrome needs to drive it (e.g. a hamburger button living
 * in some other header). Reads the SAME slice state `TwoPanelLayout` renders
 * from, so toggles stay in sync.
 */
export function useTwoPanelLayout(id: string, defaults?: Partial<PanelLayout>) {
  const dispatch = useAppDispatch();
  const layout = useAppSelector(selectPanelLayout(id, defaults));

  const show = useCallback(
    (visible: boolean) => dispatch(setSidebarVisible({ id, visible })),
    [dispatch, id]
  );
  const toggle = useCallback(() => dispatch(toggleSidebar({ id })), [dispatch, id]);

  return {
    sidebarVisible: layout.sidebarVisible,
    sidebarWidth: layout.sidebarWidth,
    showSidebar: show,
    toggleSidebar: toggle,
  };
}

export interface TwoPanelLayoutProps {
  /** Stable id used as the persistence key for this layout's geometry. */
  id: string;
  /** Content of the mini sidebar (left pane). */
  sidebar: ReactNode;
  /** Main content (right pane). */
  children: ReactNode;
  /** Sidebar visibility on first ever mount (before any persisted state). */
  defaultSidebarVisible?: boolean;
  /** Sidebar width in px on first ever mount. */
  defaultSidebarWidth?: number;
  /** Minimum sidebar width while dragging. */
  minSidebarWidth?: number;
  /** Maximum sidebar width while dragging. */
  maxSidebarWidth?: number;
  /**
   * Force the sidebar open regardless of persisted state (e.g. an onboarding
   * lockdown where the sidebar must always show). The persisted preference is
   * untouched, so it restores once the force is lifted.
   */
  forceSidebarVisible?: boolean;
  /** Step (px) the keyboard divider moves per arrow press. */
  keyboardStep?: number;
  className?: string;
  sidebarClassName?: string;
  contentClassName?: string;
  /**
   * Shared appearance applied to BOTH panes — the card background, rounded
   * corners, border and shadow live here (not in the panes' own content) so
   * every two-pane screen gets a consistent look for free. Pass `''` to opt
   * out (e.g. a flush, borderless layout).
   */
  paneClassName?: string;
  /**
   * Show a thin rail with a reopen button when the sidebar is hidden. Defaults
   * to false because chat surfaces its own toggle in the header; standalone
   * uses can opt in.
   */
  showCollapsedRail?: boolean;
  /**
   * Show the visible grab handle on the resize divider. When false the divider
   * is still draggable (and shows a faint line on hover/focus) but renders no
   * resting holder — a cleaner look for screens that don't want the affordance
   * front-and-center. Defaults to true.
   */
  showDividerHandle?: boolean;
  /**
   * Join the two panes into a single bordered card with no gap between them: the
   * shared edge becomes a flush, hairline drag divider. This is the default for
   * every two-pane surface; pass `false` for the legacy split-card look with a
   * gutter divider (no current callers).
   */
  seamless?: boolean;
}

/** Default card look shared by both panes. */
export const DEFAULT_PANE_CLASS = 'bg-surface rounded-2xl shadow-soft border border-line';

const DEFAULT_MIN_WIDTH = 180;
const DEFAULT_MAX_WIDTH = 480;
const DEFAULT_KEYBOARD_STEP = 16;

/**
 * A reusable two-pane shell: a resizable mini sidebar on the left and main
 * content on the right. Visibility and the dragged width persist per `id` via
 * the Redux `layout` slice, so the layout is remembered across reloads.
 *
 * Resize: drag the divider between the panes (pointer) or focus it and use the
 * arrow keys. Width is clamped to [minSidebarWidth, maxSidebarWidth] and only
 * committed to the store on drag end to avoid thrashing redux-persist.
 */
export default function TwoPanelLayout({
  id,
  sidebar,
  children,
  defaultSidebarVisible = false,
  defaultSidebarWidth,
  minSidebarWidth = DEFAULT_MIN_WIDTH,
  maxSidebarWidth = DEFAULT_MAX_WIDTH,
  forceSidebarVisible = false,
  keyboardStep = DEFAULT_KEYBOARD_STEP,
  className = '',
  sidebarClassName = '',
  contentClassName = '',
  paneClassName = DEFAULT_PANE_CLASS,
  showCollapsedRail = false,
  showDividerHandle = true,
  seamless = true,
}: TwoPanelLayoutProps) {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const layout = useAppSelector(
    selectPanelLayout(id, {
      sidebarVisible: defaultSidebarVisible,
      ...(defaultSidebarWidth != null ? { sidebarWidth: defaultSidebarWidth } : {}),
    })
  );

  // Seed persisted geometry from this component's defaults exactly once per id.
  useEffect(() => {
    dispatch(
      ensurePanelLayout({
        id,
        defaults: {
          sidebarVisible: defaultSidebarVisible,
          ...(defaultSidebarWidth != null ? { sidebarWidth: defaultSidebarWidth } : {}),
        },
      })
    );
    // Intentionally only on id change — defaults are a first-mount seed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id]);

  const isOpen = forceSidebarVisible || layout.sidebarVisible;

  // Live width while dragging is kept local (and applied via inline style) so
  // we don't dispatch — and re-persist — on every pointer move.
  const [dragWidth, setDragWidth] = useState<number | null>(null);
  const dragWidthRef = useRef<number | null>(null);
  const persistedWidth = clampWidth(layout.sidebarWidth, minSidebarWidth, maxSidebarWidth);
  const width = dragWidth ?? persistedWidth;

  const commitWidth = useCallback(
    (next: number) => {
      const clamped = clampWidth(Math.round(next), minSidebarWidth, maxSidebarWidth);
      dispatch(setSidebarWidth({ id, width: clamped }));
      debug('commit width', { id, width: clamped });
    },
    [dispatch, id, minSidebarWidth, maxSidebarWidth]
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
        const next = clampWidth(
          startWidth + (ev.clientX - startX),
          minSidebarWidth,
          maxSidebarWidth
        );
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
      debug('drag start', { id, startWidth });
    },
    [width, minSidebarWidth, maxSidebarWidth, commitWidth, id]
  );

  // Detach global listeners if we unmount mid-drag.
  useLayoutEffect(() => {
    return () => {
      dragCleanupRef.current?.();
    };
  }, []);

  const onDividerKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'ArrowLeft') {
        e.preventDefault();
        commitWidth(persistedWidth - keyboardStep);
      } else if (e.key === 'ArrowRight') {
        e.preventDefault();
        commitWidth(persistedWidth + keyboardStep);
      }
    },
    [commitWidth, persistedWidth, keyboardStep]
  );

  // In seamless mode the card lives on the wrapper that holds both panes, so the
  // panes themselves carry no border/rounding and sit flush against the divider.
  const paneCard = seamless ? '' : paneClassName;

  const panes = (
    <>
      {isOpen && (
        <>
          <div
            className={`flex-shrink-0 min-w-0 overflow-hidden ${paneCard} ${sidebarClassName}`}
            style={{ width }}
            data-testid={`two-panel-sidebar-${id}`}>
            {sidebar}
          </div>

          {/* Drag handle / divider */}
          <div
            role="separator"
            aria-orientation="vertical"
            aria-label={t('layout.resizeSidebar')}
            aria-valuenow={Math.round(width)}
            aria-valuemin={minSidebarWidth}
            aria-valuemax={maxSidebarWidth}
            tabIndex={0}
            data-testid={`two-panel-divider-${id}`}
            data-analytics-id="two-panel-resize-divider"
            onPointerDown={onPointerDown}
            onKeyDown={onDividerKeyDown}
            className={
              seamless
                ? // Flush hairline seam: 1px visible line, wider invisible hit
                  // area, highlights on hover/focus.
                  'group relative w-px flex-shrink-0 cursor-col-resize select-none self-stretch bg-surface-strong focus:outline-none'
                : `group relative flex flex-shrink-0 cursor-col-resize select-none items-center justify-center self-stretch focus:outline-none ${
                    // Tighter gutter between panes when there's no visible handle.
                    showDividerHandle ? 'mx-1 w-3' : 'mx-0 w-1.5'
                  }`
            }
            title={t('layout.resizeSidebar')}>
            {seamless ? (
              <>
                {/* Wider transparent grab strip straddling the 1px seam; z-10
                    keeps it above the adjacent panes so it stays grabbable. */}
                <span className="absolute inset-y-0 -left-1 -right-1 z-10" />
                {/* The seam line itself, brightened on hover/focus. */}
                <span className="absolute inset-0 transition-colors group-hover:bg-primary-400 group-focus:bg-primary-500" />
              </>
            ) : (
              /* Transparent hit area (full height) with a short grab handle
                 centered vertically. When the handle is hidden it stays
                 transparent at rest and only surfaces on hover/focus. */
              <span
                className={`h-10 w-1 rounded-full transition-colors group-hover:bg-primary-400 group-focus:bg-primary-500 ${
                  showDividerHandle ? 'bg-stone-400 dark:bg-neutral-500' : 'bg-transparent'
                }`}
              />
            )}
          </div>
        </>
      )}

      {!isOpen && showCollapsedRail && (
        <button
          type="button"
          data-testid={`two-panel-reopen-${id}`}
          data-analytics-id="two-panel-reopen-sidebar"
          onClick={() => dispatch(setSidebarVisible({ id, visible: true }))}
          title={t('layout.showSidebar')}
          aria-label={t('layout.showSidebar')}
          className="flex-shrink-0 w-6 self-stretch flex items-center justify-center text-content-faint hover:text-primary-500 hover:bg-surface-hover transition-colors">
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
          </svg>
        </button>
      )}

      <div className={`flex-1 min-w-0 overflow-hidden ${paneCard} ${contentClassName}`}>
        {children}
      </div>
    </>
  );

  return (
    <div className={`flex min-h-0 ${className}`}>
      {seamless ? (
        <div className={`flex min-h-0 flex-1 overflow-hidden ${DEFAULT_PANE_CLASS}`}>{panes}</div>
      ) : (
        panes
      )}
    </div>
  );
}
