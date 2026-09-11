import { isMac } from '../../../lib/commands/shortcut';
import { isTauri } from '../../../utils/tauriCommands/common';

/**
 * Height (px) of the drag strip. Matches the macOS traffic-light zone so the
 * native window controls sit within the band.
 */
export const WINDOW_DRAG_BAR_HEIGHT = 28;

/**
 * Transparent macOS window-drag band for the overlay title bar.
 *
 * The main window runs with `titleBarStyle: "Overlay"` + `hiddenTitle` (see
 * `app/src-tauri/tauri.conf.json`), so macOS draws transparent traffic lights
 * over the web content but does NOT make the top draggable on its own — the
 * webview captures the pointer events. We opt back in with a `data-tauri-drag-
 * region` band.
 *
 * Positioned over the top of the content column ({@link RootShellLayout}), so
 * it does not reserve vertical space or add an inherited top inset to routed
 * pages. The window controls overlay this same title-bar region.
 *
 * Native CEF provider webviews composite above all HTML and so can't be dragged
 * through; that's a platform limit, not this band. The sidebar is intentionally
 * excluded — its header already drags in place.
 *
 * The attribute is deliberately bare (React renders it as `="true"`). `drag.js`
 * reads that as direct-hit-only — `el === composedPath[0]` — which is exactly
 * right for a band with no children, since the pointer target always *is* this
 * element. Keep it childless: a container needs `="deep"` instead, or only its
 * own uncovered box drags. `SidebarHeader` and `AppSidebar` were that bug.
 *
 * macOS-only: Windows/Linux keep their native decorated title bar (the
 * `Overlay` style is a no-op there), so reserving a band would only waste
 * vertical space. Outside the Tauri runtime (browser/iOS) there is no window to
 * drag, so it renders nothing.
 */
export default function WindowDragBar() {
  if (!isTauri() || !isMac()) return null;
  return (
    <div
      data-tauri-drag-region
      aria-hidden="true"
      className="absolute inset-x-0 top-0 z-10"
      style={{ height: WINDOW_DRAG_BAR_HEIGHT }}
    />
  );
}
