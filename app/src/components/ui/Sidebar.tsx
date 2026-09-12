import { Slot } from 'radix-ui';
import {
  type ButtonHTMLAttributes,
  createContext,
  forwardRef,
  type HTMLAttributes,
  type LiHTMLAttributes,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react';

import { cn } from '../../lib/cn';

/**
 * The app shell's sidebar, as a primitive.
 *
 * Ported from shadcn's `sidebar` (provider + collapsible column + trigger +
 * menu primitives) but hand-authored against Radix and the OpenHuman token
 * scale, and shaped to what `components/layout/shell/` actually does rather
 * than to shadcn's demo. Three differences are deliberate:
 *
 * - **A resizable rail, not a decorative one.** `RootShellLayout` drags the
 *   sidebar's width and persists it, so {@link SidebarRail} is a real
 *   `role="separator"` with pointer-drag *and* arrow-key resizing, not the
 *   click-to-toggle strip shadcn ships.
 * - **`collapsible="offcanvas"` unmounts the column** instead of animating it
 *   to zero width. The shell renders a native webview glued to the content
 *   bounds; a zero-width-but-present column still occupies layout space in
 *   ways that surface has historically punched through.
 * - **No Redux, no cookie persistence.** State lives here (uncontrolled) or in
 *   the caller (controlled via `open` / `onOpenChange`, `width` /
 *   `onWidthChange`). Wiring it to the `layout` slice is the shell's job.
 *
 * There are no user-facing strings in this file: every label
 * (`aria-label`, `title`) is supplied by the caller, which is what keeps the
 * primitive out of the i18n surface entirely.
 */

export const SIDEBAR_DEFAULT_WIDTH = 224;
export const SIDEBAR_MIN_WIDTH = 188;
export const SIDEBAR_MAX_WIDTH = 420;
/** Column width when collapsed with `collapsible="icon"` — fits a 32px rail button. */
export const SIDEBAR_ICON_WIDTH = 56;
/** Pixels per arrow-key press on {@link SidebarRail}. */
export const SIDEBAR_KEYBOARD_STEP = 16;

export type SidebarState = 'expanded' | 'collapsed';
export type SidebarSide = 'left' | 'right';
export type SidebarCollapsible = 'offcanvas' | 'icon' | 'none';

export interface SidebarContextValue {
  /** Whether the sidebar is expanded. */
  open: boolean;
  /** `open` as the string the `data-state` contract exposes. */
  state: SidebarState;
  setOpen: (next: boolean) => void;
  toggleSidebar: () => void;
  /** Current column width in px (the live drag width while dragging). */
  width: number;
  setWidth: (next: number) => void;
  minWidth: number;
  maxWidth: number;
}

const SidebarContext = createContext<SidebarContextValue | null>(null);

/**
 * Read the surrounding {@link SidebarProvider}. Throws outside one — a sidebar
 * part that silently no-ops is worse than a stack trace pointing at the
 * missing provider.
 */
export function useSidebar(): SidebarContextValue {
  const ctx = useContext(SidebarContext);
  if (!ctx) throw new Error('useSidebar must be used within a <SidebarProvider>');
  return ctx;
}

function clampWidth(width: number, min: number, max: number): number {
  return Math.min(Math.max(width, min), max);
}

/**
 * The widest the sidebar may render at, for a given viewport (#5907).
 *
 * Half the window, floored — so the column can never own more than half a
 * narrow one. Inert on any real desktop: it only bites below `2 * maxWidth`,
 * i.e. an 840px window at the default 420px cap.
 *
 * `min` wins over this on purpose. A window narrower than `2 * min` is past the
 * point where a fraction is meaningful, and returning something below `min`
 * would fight `clampWidth`'s floor and make the two disagree.
 */
function viewportCap(viewportWidth: number, min: number): number {
  return Math.max(min, Math.floor(viewportWidth / 2));
}

/**
 * The current window width, tracked across resizes.
 *
 * A listener rather than a CSS `max-width`, and the distinction is the whole
 * point (#5941, Codex). A CSS-only clamp renders the right number while every
 * consumer of the stored one — the rail's drag arithmetic, the arrow-key step,
 * `aria-valuenow` — keeps reading the larger value. With a persisted 420 in a
 * 414px viewport that renders a 207px column the rail cannot narrow at all:
 * dragging fully left proposes ~213, `max-width` still pins 207, and the
 * separator announces 420 for a 207px column. Clamping the value instead means
 * there is one width and everything agrees with it.
 *
 * `Infinity` before mount so the cap is inert until a real measurement exists —
 * a 0 here would collapse the sidebar to `min` for one frame.
 */
function useViewportWidth(): number {
  const [viewportWidth, setViewportWidth] = useState(() =>
    typeof window === 'undefined' ? Number.POSITIVE_INFINITY : window.innerWidth
  );

  useEffect(() => {
    if (typeof window === 'undefined') return;
    const onResize = () => setViewportWidth(window.innerWidth);
    // Sync once on mount: the lazy initial state above runs before any resize
    // that happened between module evaluation and this effect.
    onResize();
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);

  return viewportWidth;
}

export interface SidebarProviderProps extends HTMLAttributes<HTMLDivElement> {
  /** Controlled open state. Omit for uncontrolled. */
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  /** Controlled width in px. Omit for uncontrolled. */
  width?: number;
  defaultWidth?: number;
  /** Fires on every drag frame and on each arrow-key step. */
  onWidthChange?: (width: number) => void;
  minWidth?: number;
  maxWidth?: number;
  /**
   * Bind ⌘/Ctrl+B to the toggle. **Off by default**, unlike shadcn: this app
   * already registers that chord in its global command registry, and two
   * listeners on one chord toggle twice and cancel out.
   */
  keyboardShortcut?: boolean;
  children?: ReactNode;
}

export const SidebarProvider = forwardRef<HTMLDivElement, SidebarProviderProps>(
  (
    {
      open: openProp,
      defaultOpen = true,
      onOpenChange,
      width: widthProp,
      defaultWidth = SIDEBAR_DEFAULT_WIDTH,
      onWidthChange,
      minWidth = SIDEBAR_MIN_WIDTH,
      maxWidth = SIDEBAR_MAX_WIDTH,
      keyboardShortcut = false,
      className,
      style,
      children,
      ...rest
    },
    ref
  ) => {
    const [uncontrolledOpen, setUncontrolledOpen] = useState(defaultOpen);
    const open = openProp ?? uncontrolledOpen;

    const [uncontrolledWidth, setUncontrolledWidth] = useState(() =>
      clampWidth(defaultWidth, minWidth, maxWidth)
    );
    // One effective width, viewport included, handed to everything through
    // context: the rendered column, the rail's drag origin, its arrow-key step
    // and its `aria-valuenow`. The STORED value keeps the user's preference
    // untouched — widening the window restores it — but nothing reads the
    // stored value directly, so no consumer can disagree with what is on screen.
    const viewportWidth = useViewportWidth();
    const effectiveMax = Math.min(maxWidth, viewportCap(viewportWidth, minWidth));
    const width = clampWidth(widthProp ?? uncontrolledWidth, minWidth, effectiveMax);

    const setOpen = useCallback(
      (next: boolean) => {
        if (openProp === undefined) setUncontrolledOpen(next);
        onOpenChange?.(next);
      },
      [openProp, onOpenChange]
    );

    const toggleSidebar = useCallback(() => setOpen(!open), [open, setOpen]);

    const setWidth = useCallback(
      (next: number) => {
        // `effectiveMax`, not `maxWidth`: a drag in a narrow window must not be
        // able to STORE a width the column can never render. Clamping here
        // against the raw maximum would put the stored value and the rendered
        // one back into disagreement — the same defect one layer up.
        const clamped = clampWidth(Math.round(next), minWidth, effectiveMax);
        if (widthProp === undefined) setUncontrolledWidth(clamped);
        onWidthChange?.(clamped);
      },
      [widthProp, onWidthChange, minWidth, effectiveMax]
    );

    useEffect(() => {
      if (!keyboardShortcut) return undefined;
      const onKeyDown = (event: KeyboardEvent) => {
        if (event.key.toLowerCase() !== 'b') return;
        if (!event.metaKey && !event.ctrlKey) return;
        event.preventDefault();
        toggleSidebar();
      };
      window.addEventListener('keydown', onKeyDown);
      return () => window.removeEventListener('keydown', onKeyDown);
    }, [keyboardShortcut, toggleSidebar]);

    const value = useMemo<SidebarContextValue>(
      () => ({
        open,
        state: open ? 'expanded' : 'collapsed',
        setOpen,
        toggleSidebar,
        width,
        setWidth,
        minWidth,
        // The ACTIVE maximum, so every consumer agrees on the ceiling as well
        // as on the current value. `SidebarRail` publishes this as
        // `aria-valuemax`; exposing the raw 420 here announced a ceiling the
        // column could not reach in a narrow window.
        maxWidth: effectiveMax,
      }),
      [open, setOpen, toggleSidebar, width, setWidth, minWidth, effectiveMax]
    );

    return (
      <SidebarContext.Provider value={value}>
        <div
          ref={ref}
          data-slot="sidebar-provider"
          data-state={value.state}
          className={cn('relative flex h-full w-full min-h-0 overflow-hidden', className)}
          // Exposed as a custom property so page-level CSS can align to the
          // column without re-reading React state.
          style={{ ...(style ?? {}), ['--sidebar-width' as string]: `${width}px` }}
          {...rest}>
          {children}
        </div>
      </SidebarContext.Provider>
    );
  }
);
SidebarProvider.displayName = 'SidebarProvider';

export interface SidebarProps extends HTMLAttributes<HTMLDivElement> {
  side?: SidebarSide;
  /**
   * - `offcanvas` — collapsing unmounts the column (default).
   * - `icon` — collapsing narrows it to {@link SIDEBAR_ICON_WIDTH}.
   * - `none` — always expanded; the trigger becomes inert chrome.
   */
  collapsible?: SidebarCollapsible;
}

/** The sidebar column itself. Width comes from the provider, not from props. */
export const Sidebar = forwardRef<HTMLDivElement, SidebarProps>(
  ({ side = 'left', collapsible = 'offcanvas', className, style, children, ...rest }, ref) => {
    const { state, width } = useSidebar();
    const collapsed = state === 'collapsed' && collapsible !== 'none';

    if (collapsed && collapsible === 'offcanvas') return null;

    return (
      <div
        ref={ref}
        data-slot="sidebar"
        data-state={collapsible === 'none' ? 'expanded' : state}
        data-side={side}
        data-collapsible={collapsible}
        className={cn(
          'flex h-full min-h-0 min-w-0 flex-none flex-col overflow-hidden text-content',
          className
        )}
        style={{ width: collapsed ? SIDEBAR_ICON_WIDTH : width, ...(style ?? {}) }}
        {...rest}>
        {children}
      </div>
    );
  }
);
Sidebar.displayName = 'Sidebar';

export interface SidebarRailProps extends HTMLAttributes<HTMLDivElement> {
  /** Required for a11y — the primitive ships no strings of its own. */
  'aria-label'?: string;
  /**
   * Classes for the hover/focus seam indicator (the inner `<span>`), separate
   * from `className` (the outer `role="separator"` hit-target). Defaults to
   * `line-chrome` — every current caller paints this rail directly on the
   * chrome background, where the plain `line` token reads too faint. Override
   * this, not `className`, if a future caller sits on a regular surface.
   */
  indicatorClassName?: string;
}

const DEFAULT_RAIL_INDICATOR = 'group-hover:bg-line-chrome group-focus:bg-line-chrome';

/**
 * Draggable seam between the column and the content. Transparent at rest so it
 * does not draw a line across the chrome; it lights up on hover/focus to
 * advertise the affordance. Arrow keys resize by {@link SIDEBAR_KEYBOARD_STEP}.
 */
export const SidebarRail = forwardRef<HTMLDivElement, SidebarRailProps>(
  ({ className, indicatorClassName, onPointerDown, onKeyDown, ...rest }, ref) => {
    const { width, setWidth, minWidth, maxWidth, open } = useSidebar();
    const detachRef = useRef<(() => void) | null>(null);

    // Drop global listeners if we unmount mid-drag.
    useLayoutEffect(() => () => detachRef.current?.(), []);

    const handlePointerDown = useCallback(
      (event: ReactPointerEvent<HTMLDivElement>) => {
        onPointerDown?.(event);
        if (event.defaultPrevented) return;
        event.preventDefault();
        const startX = event.clientX;
        const startWidth = width;

        const handleMove = (moveEvent: PointerEvent) => {
          setWidth(startWidth + (moveEvent.clientX - startX));
        };
        const detach = () => {
          window.removeEventListener('pointermove', handleMove);
          window.removeEventListener('pointerup', detach);
          window.removeEventListener('pointercancel', detach);
          window.removeEventListener('blur', detach);
          detachRef.current = null;
        };

        detachRef.current = detach;
        window.addEventListener('pointermove', handleMove);
        window.addEventListener('pointerup', detach);
        window.addEventListener('pointercancel', detach);
        window.addEventListener('blur', detach);
      },
      [onPointerDown, width, setWidth]
    );

    const handleKeyDown = useCallback(
      (event: ReactKeyboardEvent<HTMLDivElement>) => {
        onKeyDown?.(event);
        if (event.defaultPrevented) return;
        if (event.key === 'ArrowLeft') {
          event.preventDefault();
          setWidth(width - SIDEBAR_KEYBOARD_STEP);
        } else if (event.key === 'ArrowRight') {
          event.preventDefault();
          setWidth(width + SIDEBAR_KEYBOARD_STEP);
        }
      },
      [onKeyDown, width, setWidth]
    );

    return (
      <div
        ref={ref}
        role="separator"
        aria-orientation="vertical"
        aria-valuenow={Math.round(width)}
        aria-valuemin={minWidth}
        aria-valuemax={maxWidth}
        tabIndex={0}
        data-slot="sidebar-rail"
        data-state={open ? 'expanded' : 'collapsed'}
        onPointerDown={handlePointerDown}
        onKeyDown={handleKeyDown}
        className={cn(
          // `w-0`, not `w-px`: the rail sits between the column and the content
          // card, so any layout width of its own makes the card's left gutter
          // wider than its other three by exactly that much. The seam is drawn
          // by the absolutely-positioned indicator below instead, which costs
          // no width — and it only paints on hover/focus anyway.
          //
          // `z-20` is load-bearing, not decoration. `SidebarInset` — the content
          // card, rendered AFTER this element by `RootShellLayout` — carries
          // `relative z-10`, and the hit area below carried `z-10` too. Equal
          // z-index means DOM order decides, so the card painted over the half
          // of the hit area that overhangs it and pointer events never reached
          // the rail there: `elementFromPoint` at the rail's own centre returned
          // the content viewport, not this element (#5906). Raising the rail
          // itself rather than the child lifts BOTH the hit area and the 1px
          // seam, which share the cause.
          'group relative z-20 w-0 flex-none cursor-col-resize select-none self-stretch',
          'bg-transparent focus:outline-hidden',
          className
        )}
        {...rest}>
        {/* Widened hit area — the visible seam stays 1px. */}
        <span className="absolute inset-y-0 -left-1 -right-1 z-10" />
        <span
          className={cn(
            'absolute inset-y-0 left-0 w-px transition-colors',
            indicatorClassName ?? DEFAULT_RAIL_INDICATOR
          )}
        />
      </div>
    );
  }
);
SidebarRail.displayName = 'SidebarRail';

export interface SidebarTriggerProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  /** Render the caller's own element instead of a `<button>`. */
  asChild?: boolean;
}

/** Toggles the sidebar. Pass an `aria-label` — this primitive ships no copy. */
export const SidebarTrigger = forwardRef<HTMLButtonElement, SidebarTriggerProps>(
  ({ asChild = false, className, onClick, children, ...rest }, ref) => {
    const { toggleSidebar, state } = useSidebar();
    const Comp = asChild ? Slot.Root : 'button';
    return (
      <Comp
        ref={ref}
        type={asChild ? undefined : 'button'}
        data-slot="sidebar-trigger"
        data-state={state}
        onClick={(event: React.MouseEvent<HTMLButtonElement>) => {
          onClick?.(event);
          if (event.defaultPrevented) return;
          toggleSidebar();
        }}
        className={cn(
          'inline-flex h-7 w-7 flex-none items-center justify-center rounded-md',
          'text-content-muted transition-colors hover:bg-surface-hover hover:text-content-secondary',
          'focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-500/25',
          className
        )}
        {...rest}>
        {children}
      </Comp>
    );
  }
);
SidebarTrigger.displayName = 'SidebarTrigger';

/**
 * The routed content beside the column. `unframed` renders it edge-to-edge —
 * the framed default is an inset, rounded card on the chrome.
 *
 * The card's hairline is painted by an `::after` overlay rather than by the
 * element's own `box-shadow`. An inset shadow renders above the element's
 * background but *below* its children's, so any page whose root paints an
 * opaque fill — `Thread`'s `bg-background` on the chat surface is the one that
 * exposed this — covered the edge and the page appeared borderless. The
 * overlay sits above the content, is `pointer-events-none`, and inherits the
 * radius, so it draws the same hairline on every page regardless of what the
 * page paints.
 */
export interface SidebarInsetProps extends HTMLAttributes<HTMLDivElement> {
  unframed?: boolean;
}

export const SidebarInset = forwardRef<HTMLDivElement, SidebarInsetProps>(
  ({ unframed = false, className, ...rest }, ref) => (
    <div
      ref={ref}
      data-slot="sidebar-inset"
      data-unframed={unframed ? 'true' : undefined}
      className={cn(
        'relative z-10 flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-surface',
        // `my-3 mr-3`, with NO left margin: the card butts against the sidebar
        // column so the two read as one continuous surface, and the gutter the
        // shell shows is the three outer edges. A left margin here put a strip
        // of chrome between the nav and the content that nothing else lined up
        // with.
        !unframed &&
          'my-3 mr-3 rounded-2xl after:pointer-events-none after:absolute after:inset-0 after:z-20 after:rounded-[inherit] after:shadow-content-edge',
        className
      )}
      {...rest}
    />
  )
);
SidebarInset.displayName = 'SidebarInset';

export const SidebarHeader = forwardRef<HTMLDivElement, HTMLAttributes<HTMLDivElement>>(
  ({ className, ...rest }, ref) => (
    <div
      ref={ref}
      data-slot="sidebar-header"
      className={cn('flex flex-none flex-col gap-1 px-3 pb-2 pt-3', className)}
      {...rest}
    />
  )
);
SidebarHeader.displayName = 'SidebarHeader';

/** Scrolling middle band. Everything above and below it is pinned. */
export const SidebarContent = forwardRef<HTMLDivElement, HTMLAttributes<HTMLDivElement>>(
  ({ className, ...rest }, ref) => (
    <div
      ref={ref}
      data-slot="sidebar-content"
      className={cn('flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto', className)}
      {...rest}
    />
  )
);
SidebarContent.displayName = 'SidebarContent';

export const SidebarFooter = forwardRef<HTMLDivElement, HTMLAttributes<HTMLDivElement>>(
  ({ className, ...rest }, ref) => (
    <div
      ref={ref}
      data-slot="sidebar-footer"
      className={cn('flex flex-none flex-col gap-1 px-3 pb-2 pt-1', className)}
      {...rest}
    />
  )
);
SidebarFooter.displayName = 'SidebarFooter';

export const SidebarSeparator = forwardRef<HTMLDivElement, HTMLAttributes<HTMLDivElement>>(
  ({ className, ...rest }, ref) => (
    <div
      ref={ref}
      role="separator"
      aria-orientation="horizontal"
      data-slot="sidebar-separator"
      className={cn('mx-2 h-px flex-none bg-line-subtle', className)}
      {...rest}
    />
  )
);
SidebarSeparator.displayName = 'SidebarSeparator';

export const SidebarGroup = forwardRef<HTMLDivElement, HTMLAttributes<HTMLDivElement>>(
  ({ className, ...rest }, ref) => (
    <div
      ref={ref}
      data-slot="sidebar-group"
      className={cn('relative flex w-full min-w-0 flex-col px-3 py-1', className)}
      {...rest}
    />
  )
);
SidebarGroup.displayName = 'SidebarGroup';

export interface SidebarGroupLabelProps extends HTMLAttributes<HTMLDivElement> {
  asChild?: boolean;
}

export const SidebarGroupLabel = forwardRef<HTMLDivElement, SidebarGroupLabelProps>(
  ({ asChild = false, className, ...rest }, ref) => {
    const Comp = asChild ? Slot.Root : 'div';
    return (
      <Comp
        ref={ref}
        data-slot="sidebar-group-label"
        className={cn(
          // px-2.5 matches SidebarMenuButton's own inner padding, so a group
          // heading's text sits on the same left edge as the row labels under it.
          'flex h-7 shrink-0 items-center px-2.5 text-micro font-medium uppercase tracking-wide text-content-faint',
          className
        )}
        {...rest}
      />
    );
  }
);
SidebarGroupLabel.displayName = 'SidebarGroupLabel';

export const SidebarGroupContent = forwardRef<HTMLDivElement, HTMLAttributes<HTMLDivElement>>(
  ({ className, ...rest }, ref) => (
    <div
      ref={ref}
      data-slot="sidebar-group-content"
      className={cn('flex w-full min-w-0 flex-col', className)}
      {...rest}
    />
  )
);
SidebarGroupContent.displayName = 'SidebarGroupContent';

export const SidebarMenu = forwardRef<HTMLUListElement, HTMLAttributes<HTMLUListElement>>(
  ({ className, ...rest }, ref) => (
    <ul
      ref={ref}
      data-slot="sidebar-menu"
      className={cn('flex w-full min-w-0 flex-col gap-0.5', className)}
      {...rest}
    />
  )
);
SidebarMenu.displayName = 'SidebarMenu';

export const SidebarMenuItem = forwardRef<HTMLLIElement, LiHTMLAttributes<HTMLLIElement>>(
  ({ className, ...rest }, ref) => (
    <li
      ref={ref}
      data-slot="sidebar-menu-item"
      className={cn('group/menu-item relative list-none', className)}
      {...rest}
    />
  )
);
SidebarMenuItem.displayName = 'SidebarMenuItem';

export type SidebarMenuButtonSize = 'sm' | 'md' | 'lg';

export interface SidebarMenuButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  asChild?: boolean;
  /** Current-route highlight. Also sets `aria-current="page"`. */
  isActive?: boolean;
  size?: SidebarMenuButtonSize;
}

const MENU_BUTTON_SIZES: Record<SidebarMenuButtonSize, string> = {
  sm: 'h-7 gap-2 px-2 text-xs',
  md: 'h-9 gap-2.5 px-2.5 text-sm',
  lg: 'h-11 gap-3 px-3 text-base',
};

/**
 * One navigation row.
 *
 * The active state is a solid `primary-500` pill with inverted text. This was
 * a neutral `bg-surface/70` fill, on the reasoning that the chrome carries the
 * theme's hue so an accent pill stacks two colours. That held while the chrome
 * was a themed WebGL mesh; the default backdrop is flat now and the chrome is a
 * plain neutral, so a neutral pill on it is the thing that reads as noise —
 * there is nothing for it to lift against. The accent is the only colour in
 * this column, which is what makes it legible as *selection* rather than
 * decoration.
 *
 * The hover/idle states stay alpha-based so they still lift against whatever a
 * theme paints behind them.
 */
export const SidebarMenuButton = forwardRef<HTMLButtonElement, SidebarMenuButtonProps>(
  (
    {
      asChild = false,
      isActive = false,
      size = 'md',
      className,
      'aria-current': ariaCurrent,
      ...rest
    },
    ref
  ) => {
    const Comp = asChild ? Slot.Root : 'button';
    return (
      <Comp
        ref={ref}
        type={asChild ? undefined : 'button'}
        data-slot="sidebar-menu-button"
        data-size={size}
        data-active={isActive ? 'true' : 'false'}
        aria-current={ariaCurrent ?? (isActive ? 'page' : undefined)}
        className={cn(
          'group/menu-button flex w-full min-w-0 cursor-pointer items-center rounded-md',
          'transition-colors focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-500/25',
          'disabled:pointer-events-none disabled:opacity-50',
          MENU_BUTTON_SIZES[size],
          isActive
            ? 'bg-primary-500 font-semibold text-content-inverted'
            : 'text-content-muted hover:bg-surface/40 hover:text-content-secondary',
          className
        )}
        {...rest}
      />
    );
  }
);
SidebarMenuButton.displayName = 'SidebarMenuButton';

export interface SidebarMenuBadgeProps extends HTMLAttributes<HTMLSpanElement> {
  /** Coral count pill (unread) instead of the quiet default. */
  tone?: 'default' | 'attention';
}

export const SidebarMenuBadge = forwardRef<HTMLSpanElement, SidebarMenuBadgeProps>(
  ({ tone = 'default', className, ...rest }, ref) => (
    <span
      ref={ref}
      data-slot="sidebar-menu-badge"
      data-variant={tone}
      className={cn(
        'ml-auto inline-flex h-[13px] min-w-[13px] select-none items-center justify-center',
        'rounded-full px-1 text-[9px] font-bold leading-none',
        tone === 'attention'
          ? 'bg-coral-500 text-content-inverted'
          : 'bg-surface-strong text-content-secondary',
        className
      )}
      {...rest}
    />
  )
);
SidebarMenuBadge.displayName = 'SidebarMenuBadge';

/** Fixed-size icon slot so labels align whether or not a row has an icon. */
export const SidebarMenuIcon = forwardRef<HTMLSpanElement, HTMLAttributes<HTMLSpanElement>>(
  ({ className, ...rest }, ref) => (
    <span
      ref={ref}
      data-slot="sidebar-menu-icon"
      className={cn(
        'relative inline-flex h-4 w-4 flex-none items-center justify-center',
        className
      )}
      {...rest}
    />
  )
);
SidebarMenuIcon.displayName = 'SidebarMenuIcon';

/** Truncating label. Kept separate so the icon-collapsed rail can hide it. */
export const SidebarMenuLabel = forwardRef<HTMLSpanElement, HTMLAttributes<HTMLSpanElement>>(
  ({ className, ...rest }, ref) => (
    <span
      ref={ref}
      data-slot="sidebar-menu-label"
      className={cn('min-w-0 flex-1 truncate text-left', className)}
      {...rest}
    />
  )
);
SidebarMenuLabel.displayName = 'SidebarMenuLabel';

export default Sidebar;
