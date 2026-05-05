import { type MascotFace, YellowMascot } from '../features/human/Mascot';

/**
 * Hosted inside a native macOS NSPanel + WKWebView (see
 * `app/src-tauri/src/mascot_native_window.rs`), NOT inside Tauri's runtime.
 *
 * - No `@tauri-apps/api/*` calls work here.
 * - The panel is `ignoresMouseEvents=true` so the cursor passes straight
 *   through. When the Rust host sees the cursor enter the panel frame it
 *   animates the whole NSPanel to the other right-edge corner, so the
 *   mascot bounces out of the way without going off-screen.
 * - Show/hide is driven from the tray menu in the main app.
 */
const DEFAULT_FACE: MascotFace = 'idle';

const MascotWindowApp = () => {
  return (
    <div
      style={{ position: 'fixed', inset: 0, background: 'transparent' }}
      data-face={DEFAULT_FACE}>
      <YellowMascot face={DEFAULT_FACE} groundShadowOpacity={0.75} compactArmShading />
    </div>
  );
};

export default MascotWindowApp;
