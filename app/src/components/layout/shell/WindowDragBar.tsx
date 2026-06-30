import { isMac } from '../../../lib/commands/shortcut';
import { isTauri } from '../../../utils/tauriCommands/common';

/**
 * Height (px) of the drag strip. Matches the macOS traffic-light zone so the
 * native window controls sit within the band.
 */
export const WINDOW_DRAG_BAR_HEIGHT = 28;

/**
 * Transparent macOS window-drag strip for the overlay title bar.
 *
 * The main window runs with `titleBarStyle: "Overlay"` + `hiddenTitle` (see
 * `app/src-tauri/tauri.conf.json`), so macOS draws transparent traffic lights
 * over the web content but does NOT make the top draggable on its own — the
 * webview captures the pointer events. We opt back in with a `data-tauri-drag-
 * region` strip.
 *
 * Rendered as an absolutely-positioned overlay pinned to the top of the main
 * content pane ({@link RootShellLayout}), as the LAST child so it paints ON TOP
 * of the routed view. That keeps full-bleed HTML surfaces (the Tiny Place world
 * canvas, the Chat backdrop) edge-to-edge — the strip floats over them and
 * drags the window — instead of a reserved inset that would push them down and
 * reveal the app background above them. It occupies no layout space and is
 * transparent.
 *
 * A drag region must be the top-most element under the pointer, so the band
 * does sit over the top ~28px of the pane: page chrome with controls in that
 * band keeps the bulk of each control clickable below the strip, and the
 * traffic lights (native, composited above the webview) are always clickable.
 * The sidebar is intentionally excluded — its header already drags in place.
 * Native CEF provider webviews composite above all HTML and so can't be dragged
 * through; that's a platform limit, not this strip.
 *
 * macOS-only: Windows/Linux keep their native decorated title bar (the
 * `Overlay` style is a no-op there). Outside the Tauri runtime (browser/iOS)
 * there is no window to drag, so it renders nothing.
 */
export default function WindowDragBar() {
  if (!isTauri() || !isMac()) return null;
  return (
    <div
      data-tauri-drag-region
      aria-hidden="true"
      className="absolute inset-x-0 top-0 z-20"
      style={{ height: WINDOW_DRAG_BAR_HEIGHT }}
    />
  );
}
