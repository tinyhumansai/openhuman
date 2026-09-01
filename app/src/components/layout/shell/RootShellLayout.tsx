import debugFactory from 'debug';
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
import { isTauri, safeInvoke } from '../../../utils/tauriCommands/common';
import {
  Sidebar,
  SIDEBAR_DEFAULT_WIDTH,
  SIDEBAR_MAX_WIDTH,
  SIDEBAR_MIN_WIDTH,
  SidebarProvider,
  SidebarRail,
} from '../../ui';
import ContentSurface from './ContentSurface';
import WindowDragBar from './WindowDragBar';

const log = debugFactory('sidebar');

// `app-shell` (not the older `root-shell`) so the persisted geometry seeds
// fresh with the sidebar visible by default. Exported so the global command
// layer (mod+B "Toggle sidebar") can target this exact panel.
export const APP_SHELL_LAYOUT_ID = 'app-shell';
const LAYOUT_ID = APP_SHELL_LAYOUT_ID;
// Geometry bounds come from the `Sidebar` primitive rather than being restated
// here — they were byte-identical, and two copies of a clamp is one copy too
// many once the primitive is the thing doing the clamping.
const LAYOUT_DEFAULTS = { sidebarVisible: true, sidebarWidth: SIDEBAR_DEFAULT_WIDTH };

function clamp(width: number): number {
  return Math.min(Math.max(width, SIDEBAR_MIN_WIDTH), SIDEBAR_MAX_WIDTH);
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

interface RootShellLayoutProps {
  /** Always-visible left pane (the app sidebar). */
  sidebar: ReactNode;
  /** Dynamic main content (the routed page area). */
  children: ReactNode;
  /**
   * Render the content edge-to-edge instead of as an inset card. Forwarded to
   * {@link ContentSurface} — see its docs for the compositing constraint this
   * exists for, and why no route sets it today.
   */
  unframed?: boolean;
}

/**
 * Viewport-filling two-pane shell for the app root, built as two layers rather
 * than two opaque panes:
 *
 *   - **Chrome** — this component paints nothing of its own. The themed
 *     {@link AppBackground} behind it shows through here and behind the
 *     sidebar, so the frame carries the theme's hue as one continuous surface.
 *   - **Card** — the routed content sits on a single inset, rounded
 *     {@link ContentSurface}, the only opaque sheet in the shell.
 *
 * The two separate by fill contrast — the canvas/chrome neutrals sit below the
 * card's surface — which is why the sidebar needs no border and the panes need
 * no divider fill. The dragged sidebar width persists per
 * user via the `layout` slice (id `app-shell`).
 *
 * ## Redux stays the source of truth
 *
 * The column and the rail are the `Sidebar` primitive (the collapsed-state
 * reopen affordance lives inside {@link AppSidebar}, which reads the same
 * primitive's `useSidebar()` context), driven as a **controlled view**:
 * `open` and `width` are
 * read out of the `layout` slice on every render, and `onOpenChange` /
 * `onWidthChange` dispatch back into it. Letting `SidebarProvider` hold the
 * state uncontrolled would look identical in a unit test and silently stop
 * restoring the persisted geometry across restarts.
 *
 * `keyboardShortcut` stays off (its default) for the same reason it always was:
 * `lib/commands/registry` already binds mod+B to `toggleSidebar`, and a second
 * window listener on the same chord toggles twice and cancels out.
 */
export default function RootShellLayout({ sidebar, children, unframed }: RootShellLayoutProps) {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const layout = useAppSelector(selectPanelLayout(LAYOUT_ID, LAYOUT_DEFAULTS));
  const persistedWidth = clamp(layout.sidebarWidth);
  const isOpen = layout.sidebarVisible;

  // Collapsed, the sidebar header — the app's stand-in for a title bar, and the
  // row the traffic lights are aligned to — is gone, leaving window controls
  // floating on bare content with no title anywhere. Hand the title bar back to
  // macOS for as long as the rail is collapsed, and take it back on expand.
  //
  // Driven from `isOpen` rather than from `handleOpenChange` so the window
  // agrees with a collapsed state restored from persisted layout on boot, not
  // only with states the user toggles into during this session.
  //
  // Fire-and-forget through `safeInvoke`: this is cosmetic chrome, and the
  // command is a no-op off macOS, so a failure must never break the shell
  // render. Outside Tauri there is no window at all.
  useEffect(() => {
    if (!isTauri()) return;
    void safeInvoke('set_titlebar_for_sidebar', { collapsed: !isOpen }).catch(err => {
      log('titlebar sync failed: %o', err);
    });
  }, [isOpen]);

  // Seed persisted geometry once so the selector returns a stable stored
  // reference on subsequent renders (avoids the new-object memoization warning).
  useEffect(() => {
    dispatch(ensurePanelLayout({ id: LAYOUT_ID, defaults: LAYOUT_DEFAULTS }));
  }, [dispatch]);

  // Live drag width. `SidebarRail` reports a width per pointermove frame; those
  // frames are held locally and committed to Redux once on release, so a drag
  // writes (and persists) one value rather than sixty.
  const [dragWidth, setDragWidth] = useState<number | null>(null);
  const draggingRef = useRef(false);
  const dragWidthRef = useRef<number | null>(null);
  const dragCleanupRef = useRef<(() => void) | null>(null);
  const width = dragWidth ?? persistedWidth;

  const commitWidth = useCallback(
    (next: number) => dispatch(setSidebarWidth({ id: LAYOUT_ID, width: clamp(Math.round(next)) })),
    [dispatch]
  );

  /** Every width the primitive proposes — drag frames and arrow-key steps alike. */
  const handleWidthChange = useCallback(
    (next: number) => {
      if (draggingRef.current) {
        dragWidthRef.current = next;
        setDragWidth(next);
        return;
      }
      // Arrow-key resize: discrete, so it lands straight in the store.
      commitWidth(next);
    },
    [commitWidth]
  );

  const handleOpenChange = useCallback(
    (next: boolean) => {
      log('sidebar open change: %s', next ? 'expanded' : 'collapsed');
      dispatch(setSidebarVisible({ id: LAYOUT_ID, visible: next }));
    },
    [dispatch]
  );

  // The rail owns the pointermove maths; this only brackets the gesture (and
  // paints the drag cursor across the whole window while it is in flight).
  const handleRailPointerDown = useCallback(() => {
    draggingRef.current = true;
    dragWidthRef.current = null;
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';

    function detach() {
      window.removeEventListener('pointerup', stop);
      window.removeEventListener('pointercancel', stop);
      window.removeEventListener('blur', stop);
      document.body.style.removeProperty('cursor');
      document.body.style.removeProperty('user-select');
      draggingRef.current = false;
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
    window.addEventListener('pointerup', stop);
    window.addEventListener('pointercancel', stop);
    window.addEventListener('blur', stop);
  }, [commitWidth]);

  // Detach global listeners if we unmount mid-drag.
  useLayoutEffect(() => () => dragCleanupRef.current?.(), []);

  return (
    // The chrome layer. One legibility scrim across the WHOLE shell — the
    // sidebar column and the frame around the content card — so the two read as
    // a single continuous surface. Scrimming per-pane would tint them
    // differently and reintroduce the very seam this layout removes.
    //
    // The alpha is deliberately light: the content card above is opaque, so the
    // chrome is the only place the themed AppBackground is visible at all. That
    // matters most under the opt-in `mesh` backdrop, where a heavier scrim (or a
    // backdrop blur, which also smears the 18px dotted canvas) flattens the
    // shader back into paint and leaves it burning GPU for nothing. /30 is the
    // legibility knob — raise it if sidebar labels wash out, which is most
    // likely under a `backdrop: image` theme rather than the flat default.
    <SidebarProvider
      open={isOpen}
      onOpenChange={handleOpenChange}
      width={width}
      onWidthChange={handleWidthChange}
      minWidth={SIDEBAR_MIN_WIDTH}
      maxWidth={SIDEBAR_MAX_WIDTH}
      className="bg-surface-chrome/30">
      {/* `collapsible="icon"` — a real, non-zero {@link SIDEBAR_ICON_WIDTH}px
          column that stays mounted when collapsed, rather than the previous
          `offcanvas` (unmount) + a hand-rolled sibling `<div>` standing in for
          the collapsed rail outside this column.

          `offcanvas` was chosen deliberately when this shell was built: "the
          native webview glued to the content bounds has historically punched
          through a zero-width-but-present column." That failure mode was
          CEF's per-provider child-webview architecture (`webview_accounts` /
          the CDP scanners) positioning a *separate* native webview by
          tracking a DOM placeholder's bounds — a zero-width-but-mounted
          column could desync from what the native layer painted over.

          Both halves of that architecture are gone from this codebase: the
          CDP-driven scanners and the `webview_accounts` surface they ran
          inside were removed (#5478), and the app itself moved off CEF onto
          Wry (#5456) — see `CLAUDE.md`'s Tauri-shell section. `grep -rln
          "webview_accounts\|WebviewWindow::builder" app/src-tauri/src`
          confirms there is no bounds-tracked child webview left anywhere in
          the shell; the whole app renders as one native webview, so there is
          no second compositing layer for a narrowed HTML column to be
          "punched through" by. `icon` mode's real ~48–56px column — never
          zero-width, in either state — does not reintroduce the failure mode
          `offcanvas` was chosen for; that failure mode's precondition no
          longer exists. `AppSidebar` reads {@link useSidebar}'s `state` to
          render its own compact, icon-only body while collapsed (formerly
          the sibling `<div>` here). */}
      <Sidebar collapsible="icon" data-testid="root-shell-sidebar">
        {sidebar}
      </Sidebar>

      {/* Resize seam. Transparent at rest — the sidebar and the content card
          separate by fill contrast, so a filled seam would draw a line across
          the chrome that the two-layer look is trying to remove. Arrow keys
          resize in 16px steps; the pointer drag is bracketed above. Hidden
          while collapsed — the icon-width column is fixed, not draggable. */}
      {isOpen && (
        <SidebarRail
          aria-label={t('layout.resizeSidebar')}
          title={t('layout.resizeSidebar')}
          data-testid="root-shell-divider"
          data-analytics-id="root-shell-resize-divider"
          onPointerDown={handleRailPointerDown}
        />
      )}

      <div
        className="relative flex min-w-0 flex-1 flex-col overflow-hidden"
        data-testid="root-shell-content">
        {/* macOS overlay-title-bar drag region. It is absolutely positioned, so
            the routed surface keeps its full height. No-op off macOS / outside
            Tauri, where the native title bar already owns this area. */}
        <WindowDragBar />
        <ContentSurface unframed={unframed}>{children}</ContentSurface>
      </div>
    </SidebarProvider>
  );
}
