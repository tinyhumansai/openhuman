import { type ReactNode, useCallback, useEffect } from 'react';
import { LuChevronRight } from 'react-icons/lu';

import { cn } from '../../lib/cn';
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
import { IS_DEV } from '../../utils/config';
import { Button } from '../ui';
import { clampWidth, useResizableDivider } from './useResizableDivider';

const namespace = 'two-panel-layout';

function debug(message: string, payload?: Record<string, unknown>) {
  if (IS_DEV) {
    console.debug(`[${namespace}] ${message}`, payload ?? {});
  }
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

interface TwoPanelLayoutProps {
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
const DEFAULT_PANE_CLASS = 'bg-surface rounded-2xl shadow-soft border border-line';

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
 *
 * ---
 *
 * CONVERGENCE DECISION (kept deliberately separate from `ui/Sidebar.tsx`):
 * this stays its own panel system rather than being rebuilt on
 * `SidebarProvider`/`Sidebar`/`SidebarRail`. Both are defensible in the
 * abstract — the shell sidebar is app-level chrome, this is an in-page
 * splitter — but the concrete reason is a real behavioral mismatch, not just
 * "different enough to leave alone":
 *
 * 1. **Persistence cadence is opposite by contract.** `SidebarRail`'s
 *    `onWidthChange` fires on every pointermove frame (its own doc comment
 *    says so — the shell mirrors width into a live `--sidebar-width` CSS var
 *    for chrome reflow). This component commits to Redux **once**, on
 *    pointer-up or a keyboard step, specifically to avoid thrashing
 *    redux-persist. Building this on `SidebarRail` as-is would mean
 *    dispatching on every drag frame; keeping the current commit-on-release
 *    contract would mean not using `SidebarRail`'s own callback at all —
 *    either way the primitive isn't actually doing the persistence work.
 * 2. **Visual grammar differs.** `Sidebar` assumes a single always-left
 *    column composed with `SidebarInset` (a `m-3 rounded-2xl` inset card).
 *    This component's default `seamless` mode joins BOTH panes into one
 *    bordered card with a flush 1px hairline seam — there's no `Sidebar`
 *    equivalent of that, and forcing it would mean reintroducing bespoke
 *    layout around the primitive anyway.
 * 3. **Extra affordances with no `Sidebar` counterpart**: `showCollapsedRail`
 *    (a reopen button occupying the seam when collapsed) and `seamless`
 *    itself aren't expressible through `Sidebar`'s `collapsible` variants.
 *
 * What IS shared: the clamp + pointer-drag + keyboard-step *mechanics* now
 * live in `useResizableDivider` (this directory) rather than inline in this
 * component, specifically so they're one named, independently readable unit
 * instead of hand-rolled logic duplicated alongside `SidebarRail`'s own
 * (different-contract) version. See that hook's doc comment for the full
 * reasoning on why it isn't literally the same code as `SidebarRail`.
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

  const persistedWidth = clampWidth(layout.sidebarWidth, minSidebarWidth, maxSidebarWidth);

  const commitWidth = useCallback(
    (clamped: number) => {
      dispatch(setSidebarWidth({ id, width: clamped }));
      debug('commit width', { id, width: clamped });
    },
    [dispatch, id]
  );

  // Drag/keyboard mechanics (clamp + commit-on-release) live in a shared hook
  // — see `useResizableDivider` for why this isn't the same code as the shell
  // sidebar's `SidebarRail`.
  const {
    dragWidth,
    onPointerDown,
    onKeyDown: onDividerKeyDown,
  } = useResizableDivider({
    width: persistedWidth,
    minWidth: minSidebarWidth,
    maxWidth: maxSidebarWidth,
    keyboardStep,
    onCommit: commitWidth,
  });

  // Live width while dragging is kept local (and applied via inline style) so
  // we don't dispatch — and re-persist — on every pointer move.
  const width = dragWidth ?? persistedWidth;

  // In seamless mode the card lives on the wrapper that holds both panes, so the
  // panes themselves carry no border/rounding and sit flush against the divider.
  const paneCard = seamless ? '' : paneClassName;

  const panes = (
    <>
      {isOpen && (
        <>
          <div
            className={cn('shrink-0 min-w-0 overflow-hidden', paneCard, sidebarClassName)}
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
                  'group relative w-px shrink-0 cursor-col-resize select-none self-stretch bg-surface-strong focus:outline-hidden'
                : cn(
                    'group relative flex shrink-0 cursor-col-resize select-none items-center justify-center self-stretch focus:outline-hidden',
                    // Tighter gutter between panes when there's no visible handle.
                    showDividerHandle ? 'mx-1 w-3' : 'mx-0 w-1.5'
                  )
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
                className={cn(
                  'h-10 w-1 rounded-full transition-colors group-hover:bg-primary-400 group-focus:bg-primary-500',
                  // `line-strong` rather than a raw grey pair: the token
                  // already carries the per-theme value the two palette classes
                  // were hand-picking, so the handle tracks the theme.
                  showDividerHandle ? 'bg-line-strong' : 'bg-transparent'
                )}
              />
            )}
          </div>
        </>
      )}

      {!isOpen && showCollapsedRail && (
        <Button
          variant="tertiary"
          iconOnly
          data-testid={`two-panel-reopen-${id}`}
          analyticsId="two-panel-reopen-sidebar"
          onClick={() => dispatch(setSidebarVisible({ id, visible: true }))}
          title={t('layout.showSidebar')}
          aria-label={t('layout.showSidebar')}
          // A full-height 24px seam grip rather than a button footprint: the
          // height, width and square corners are overridden, the focus ring and
          // hover fill come from Button.
          className="h-auto w-6 shrink-0 self-stretch rounded-none text-content-faint hover:text-primary-500">
          <LuChevronRight className="w-4 h-4" aria-hidden />
        </Button>
      )}

      <div className={cn('flex-1 min-w-0 overflow-hidden', paneCard, contentClassName)}>
        {children}
      </div>
    </>
  );

  return (
    <div className={cn('flex min-h-0', className)}>
      {seamless ? (
        <div className={cn('flex min-h-0 flex-1 overflow-hidden', DEFAULT_PANE_CLASS)}>{panes}</div>
      ) : (
        panes
      )}
    </div>
  );
}
