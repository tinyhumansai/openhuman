import { isMac } from '../../../lib/commands/shortcut';
import { isTauri } from '../../../utils/tauriCommands/common';
import { SIDEBAR_MAX_WIDTH } from '../../ui';
import { isWindowsDesktop, WINDOWS_WINDOW_CONTROLS_WIDTH } from './WindowsWindowControls';

/**
 * Height (px) of the drag strip. Matches the macOS traffic-light zone so the
 * native window controls sit within the band.
 */
export const WINDOW_DRAG_BAR_HEIGHT = 32;
const GLOBAL_DRAG_BAR_LEFT = SIDEBAR_MAX_WIDTH + 8;

/**
 * Transparent macOS window-drag band for the overlay title bar.
 *
 * The main window runs with `titleBarStyle: "Overlay"` + `hiddenTitle` (see
 * `crates/openhuman-app/tauri.conf.json`), so macOS draws transparent traffic lights
 * over the web content but does NOT make the top draggable on its own — the
 * webview captures the pointer events. We opt back in with a `data-tauri-drag-
 * region` band.
 *
 * Absolutely overlaid above every desktop screen, including boot and error
 * states. It contributes no layout height and paints nothing. Its left edge
 * clears the sidebar's maximum width so it never blocks sidebar buttons.
 *
 * Native child webviews composite above HTML and cannot be dragged through;
 * that is a platform limit, not this band. The sidebar is intentionally
 * excluded because its header already drags in place.
 *
 * The attribute is deliberately bare (React renders it as `="true"`). `drag.js`
 * reads that as direct-hit-only — `el === composedPath[0]` — which is exactly
 * right for a band with no children, since the pointer target always *is* this
 * element. Keep it childless: a container needs `="deep"` instead, or only its
 * own uncovered box drags. `SidebarHeader` and `AppSidebar` were that bug.
 *
 * Windows uses the same drag band with room left for its custom controls.
 * Linux keeps its native decorated title bar. Outside Tauri it renders nothing.
 */
export default function WindowDragBar() {
  if (!isTauri() || (!isMac() && !isWindowsDesktop())) return null;
  const windows = isWindowsDesktop();
  return (
    <div
      data-tauri-drag-region
      aria-hidden="true"
      className="absolute top-0 z-50 bg-transparent"
      style={{
        height: WINDOW_DRAG_BAR_HEIGHT,
        left: GLOBAL_DRAG_BAR_LEFT,
        right: windows ? WINDOWS_WINDOW_CONTROLS_WIDTH : 0,
      }}
    />
  );
}
