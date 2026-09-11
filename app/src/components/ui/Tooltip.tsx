import debug from 'debug';
import {
  cloneElement,
  type FocusEvent,
  isValidElement,
  type MouseEvent,
  type ReactElement,
  useCallback,
  useEffect,
  useRef,
  useState,
} from 'react';
import { createPortal } from 'react-dom';

const log = debug('ui:tooltip');

/** Which edge of the trigger the tooltip floats from. Sidebar icons use `right`. */
type TooltipSide = 'right' | 'top' | 'bottom' | 'left';
type TooltipAlign = 'start' | 'center' | 'end';

interface TooltipProps {
  /** Short, minimal label — e.g. "Wallet", "Settings". */
  label: string;
  /** Single focusable trigger (a button/anchor). Its hover/focus drives the tip. */
  children: ReactElement;
  /** Edge to float from. Defaults to `right` (best for a vertical sidebar rail). */
  side?: TooltipSide;
  /** Hover/focus dwell before showing, in ms. Keeps the tip from flickering. */
  delayMs?: number;
  /** Allow explanatory copy to wrap instead of using the compact one-line pill. */
  multiline?: boolean;
  /** Alignment along the selected edge. */
  align?: TooltipAlign;
}

/** Gap in px between the trigger and the tooltip pill. */
const GAP = 8;

interface Anchor {
  top: number;
  left: number;
  side: TooltipSide;
  align: TooltipAlign;
}

function anchorFor(rect: DOMRect, side: TooltipSide, align: TooltipAlign): Anchor {
  const horizontal =
    align === 'start' ? rect.left : align === 'end' ? rect.right : rect.left + rect.width / 2;
  const vertical =
    align === 'start' ? rect.top : align === 'end' ? rect.bottom : rect.top + rect.height / 2;
  switch (side) {
    case 'top':
      return { top: rect.top - GAP, left: horizontal, side, align };
    case 'bottom':
      return { top: rect.bottom + GAP, left: horizontal, side, align };
    case 'left':
      return { top: vertical, left: rect.left - GAP, side, align };
    case 'right':
    default:
      return { top: vertical, left: rect.right + GAP, side, align };
  }
}

/** Maps the float edge to the transform that pins the pill against that edge. */
const TRANSFORM: Record<TooltipSide, string> = {
  right: 'translateY(-50%)',
  left: 'translate(-100%, -50%)',
  top: 'translate(-50%, -100%)',
  bottom: 'translate(-50%, 0)',
};

function transformFor(side: TooltipSide, align: TooltipAlign): string {
  if (side === 'top') {
    return align === 'start'
      ? 'translate(0, -100%)'
      : align === 'end'
        ? 'translate(-100%, -100%)'
        : TRANSFORM[side];
  }
  if (side === 'bottom') {
    return align === 'start'
      ? 'translate(0, 0)'
      : align === 'end'
        ? 'translate(-100%, 0)'
        : TRANSFORM[side];
  }
  if (side === 'left') {
    return align === 'start'
      ? 'translate(-100%, 0)'
      : align === 'end'
        ? 'translate(-100%, -100%)'
        : TRANSFORM[side];
  }
  return align === 'start'
    ? 'translate(0, 0)'
    : align === 'end'
      ? 'translate(0, -100%)'
      : TRANSFORM[side];
}

/**
 * Lightweight, dependency-free hover/focus tooltip for icon-only controls.
 *
 * Renders a styled pill into a body portal (so it escapes the sidebar's
 * `overflow` clipping) positioned from the trigger's bounding rect. The pill
 * gives fast (~`delayMs`), on-brand feedback that the native `title` attribute
 * (lags ~1.5s, unstyled, easy to miss) cannot.
 *
 * The trigger also keeps a native `title={label}` fallback (unless it already
 * sets one). This is deliberate: the pill lives in the HTML layer, but account
 * webviews are native CEF views composited *above* HTML (see `Accounts.tsx` /
 * `RootShellLayout.tsx`), so a pill that lands over an active webview is painted
 * behind it. The OS-drawn `title` renders above everything and guarantees a
 * label survives in that case. Pair with an `aria-label` on the trigger for
 * screen readers (it takes precedence over `title`, so there's no double
 * announcement); the pill itself is decorative (`aria-hidden`).
 */
export default function Tooltip({
  label,
  children,
  side = 'right',
  delayMs = 300,
  multiline = false,
  align = 'center',
}: TooltipProps) {
  const [anchor, setAnchor] = useState<Anchor | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const clearTimer = useCallback(() => {
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  const show = useCallback(
    (rect: DOMRect) => {
      clearTimer();
      timerRef.current = setTimeout(() => {
        log('show', label);
        setAnchor(anchorFor(rect, side, align));
      }, delayMs);
    },
    [align, clearTimer, delayMs, label, side]
  );

  const hide = useCallback(() => {
    clearTimer();
    setAnchor(null);
  }, [clearTimer]);

  // Drop any pending show if the trigger unmounts mid-hover (e.g. navigation
  // tears down the rail before mouseleave/blur fires).
  useEffect(() => clearTimer, [clearTimer]);

  if (!isValidElement(children)) {
    log('children is not a valid element; rendering trigger as-is');
    return children;
  }

  const triggerProps = children.props as {
    title?: string;
    onMouseEnter?: (e: MouseEvent<HTMLElement>) => void;
    onMouseLeave?: (e: MouseEvent<HTMLElement>) => void;
    onFocus?: (e: FocusEvent<HTMLElement>) => void;
    onBlur?: (e: FocusEvent<HTMLElement>) => void;
  };

  const trigger = cloneElement(children, {
    // Native `title` fallback for when the portal pill is occluded by a native
    // CEF webview composited above the HTML layer. A trigger-supplied title wins.
    title: triggerProps.title ?? label,
    onMouseEnter: (e: MouseEvent<HTMLElement>) => {
      show(e.currentTarget.getBoundingClientRect());
      triggerProps.onMouseEnter?.(e);
    },
    onMouseLeave: (e: MouseEvent<HTMLElement>) => {
      hide();
      triggerProps.onMouseLeave?.(e);
    },
    onFocus: (e: FocusEvent<HTMLElement>) => {
      show(e.currentTarget.getBoundingClientRect());
      triggerProps.onFocus?.(e);
    },
    onBlur: (e: FocusEvent<HTMLElement>) => {
      hide();
      triggerProps.onBlur?.(e);
    },
  } as Partial<typeof children.props>);

  return (
    <>
      {trigger}
      {anchor &&
        createPortal(
          <div
            data-testid="tooltip"
            aria-hidden="true"
            // Semantic tokens, not `bg-stone-800`/`text-white`/`dark:bg-neutral-700`:
            // those are raw palette scales, so the pill kept a fixed grey under
            // every user theme instead of following it.
            className={`pointer-events-none fixed z-9999 rounded-md bg-content px-2 py-1 text-xs font-medium text-surface shadow-medium animate-fade-in ${
              multiline ? 'max-w-xs whitespace-normal' : 'whitespace-nowrap'
            }`}
            style={{
              top: anchor.top,
              left: anchor.left,
              transform: transformFor(anchor.side, anchor.align),
            }}>
            {label}
          </div>,
          document.body
        )}
    </>
  );
}
