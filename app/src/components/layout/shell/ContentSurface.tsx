import debugFactory from 'debug';
import type { ReactNode } from 'react';

import { SidebarInset } from '../../ui';

const log = debugFactory('shell:content-surface');

interface ContentSurfaceProps {
  children: ReactNode;
  /**
   * Render edge-to-edge with square corners and no card seam.
   *
   * Written for native CEF provider webviews: `WebviewHost` handed the Rust
   * side a plain `{x, y, width, height}` rectangle and CEF composited that
   * child view *above* the whole HTML layer, so its corners could not be masked
   * by `overflow-hidden`, a CSS radius, or any HTML overlay — a framed card
   * under a live webview showed four square corners punching through.
   *
   * That surface was removed upstream (`WebviewHost` is gone), so **nothing
   * sets this today**. Kept because the constraint recurs for any content the
   * compositor draws above the HTML layer, and because a full-bleed page is a
   * reasonable thing to want from a layout primitive.
   */
  unframed?: boolean;
}

/**
 * The app's single content surface: the opaque sheet that routed pages render
 * onto, floating on the themed chrome layer ({@link AppBackground}) that the
 * sidebar also sits on.
 *
 * This is the "card" half of the two-layer shell. The chrome carries the
 * theme's hue; this surface stays neutral so page content reads against a
 * consistent background across every theme.
 *
 * The geometry is the `SidebarInset` primitive: the framed default is the same
 * even 12px inset, 2xl radius and `content-edge` inset-shadow-sm this file used to
 * spell out, and `unframed` maps onto the primitive's own flag. The primitive
 * reads no sidebar context, so this stays renderable outside a
 * `SidebarProvider` — several page-level tests mount it on its own.
 */
export default function ContentSurface({ children, unframed = false }: ContentSurfaceProps) {
  log('render: unframed=%s', unframed);
  return (
    <SidebarInset unframed={unframed} data-testid="app-content-surface">
      {children}
    </SidebarInset>
  );
}
