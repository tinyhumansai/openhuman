import debugFactory from 'debug';
import { createContext, type ReactNode, useContext, useState } from 'react';
import { createPortal } from 'react-dom';

import { IS_DEV } from '../../../utils/config';

const debug = debugFactory('layout:sidebar-slot');

/**
 * Portal-based plumbing for the root shell's *dynamic* sidebar region.
 *
 * The shell renders {@link SidebarSlotOutlet} once (the middle band of
 * `AppSidebar`, between the static nav and the account menu). Any routed page
 * can then render {@link SidebarContent} to project its own sidebar UI into
 * that region. Because it's a React portal, the projected content stays inside
 * the page's own component tree — it keeps the page's context, hooks and local
 * state — while rendering into the sidebar's DOM node.
 *
 * Routes that don't render `SidebarContent` simply leave the region empty.
 */
interface SidebarSlotContextValue {
  /** The live DOM node of the dynamic region, or null before first mount. */
  target: HTMLElement | null;
  /** Stable setter (a `useState` dispatch) registering the region's node. */
  setTarget: (el: HTMLElement | null) => void;
}

const SidebarSlotContext = createContext<SidebarSlotContextValue | null>(null);

export function SidebarSlotProvider({ children }: { children: ReactNode }) {
  const [target, setTarget] = useState<HTMLElement | null>(null);
  return (
    <SidebarSlotContext.Provider value={{ target, setTarget }}>
      {children}
    </SidebarSlotContext.Provider>
  );
}

/**
 * Marks where dynamic sidebar content lands. Rendered once by `AppSidebar`.
 * The ref handler is the stable `useState` dispatch, so React attaches/detaches
 * it exactly once (no per-render portal thrash).
 */
export function SidebarSlotOutlet({ className }: { className?: string }) {
  const ctx = useContext(SidebarSlotContext);
  if (!ctx) {
    if (IS_DEV) {
      debug('SidebarSlotOutlet rendered outside SidebarSlotProvider');
    }
    return null;
  }
  return <div ref={ctx.setTarget} className={className} data-testid="sidebar-slot-outlet" />;
}

/**
 * Rendered by a routed page to inject content into the shell's dynamic sidebar
 * region. Renders nothing until the outlet has mounted (first paint), then
 * portals its children there. Safe to render when no provider is present
 * (returns null) so pages stay usable in isolated tests.
 */
export function SidebarContent({ children }: { children: ReactNode }) {
  const ctx = useContext(SidebarSlotContext);
  if (!ctx?.target) return null;
  return createPortal(children, ctx.target);
}
