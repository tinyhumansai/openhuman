import { useEffect, useRef, useState } from 'react';

import { YellowMascot } from '../features/human/Mascot';
import type { MascotFace } from '../features/human/Mascot/Ghosty';

/**
 * Hosted inside a native macOS NSPanel + WKWebView (see
 * `app/src-tauri/src/mascot_native_window.rs`), NOT inside Tauri's runtime.
 *
 * - No `@tauri-apps/api/*` calls work here.
 * - The panel is `ignoresMouseEvents=true` so the cursor passes straight
 *   through. When the Rust host sees the cursor enter the panel frame it
 *   dispatches a `mascot:hover-state` CustomEvent with `detail.hovering`.
 * - Default state is `sleep` (closed eyes). On hover the face switches to
 *   `idle`. After the cursor leaves, a 2-second timer returns to `sleep`.
 * - Show/hide is driven from the tray menu in the main app.
 *
 * [ui-flow] mascot-window: sleep → idle (hover-start) → sleep (hover-end +2s)
 */

const SLEEP_DELAY_MS = 2000;

const MascotWindowApp = () => {
  const [face, setFace] = useState<MascotFace>('sleep');
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    const handler = (e: Event) => {
      const { hovering } = (e as CustomEvent<{ hovering: boolean }>).detail;
      if (hovering) {
        if (timeoutRef.current) {
          clearTimeout(timeoutRef.current);
          timeoutRef.current = null;
        }
        setFace('idle');
        // [ui-flow] mascot-window: hover-start → face=idle
      } else {
        timeoutRef.current = setTimeout(() => {
          setFace('sleep');
          timeoutRef.current = null;
          // [ui-flow] mascot-window: sleep-delay elapsed → face=sleep
        }, SLEEP_DELAY_MS);
      }
    };

    window.addEventListener('mascot:hover-state', handler);
    return () => {
      window.removeEventListener('mascot:hover-state', handler);
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
    };
  }, []);

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'transparent' }} data-face={face}>
      <YellowMascot face={face} groundShadowOpacity={0.75} compactArmShading />
    </div>
  );
};

export default MascotWindowApp;
