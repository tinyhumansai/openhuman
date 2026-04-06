import { useEffect, useRef } from 'react';

import { Gradient } from '../lib/meshGradient';

/**
 * Animated WebGL mesh gradient background (Stripe-style).
 * Renders behind the dotted-canvas overlay so dots remain visible on top.
 * Catches WebGL errors gracefully so the app still works when the GPU context
 * is unavailable or lost (e.g. Tauri WebView on some platforms).
 */
export default function MeshGradient() {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    let gradient: InstanceType<typeof Gradient> | null = null;

    try {
      gradient = new Gradient();
      gradient.initGradient('#mesh-gradient');
    } catch (err) {
      console.warn('[MeshGradient] WebGL init failed, gradient disabled:', err);
      gradient = null;
    }

    return () => {
      try {
        if (gradient) {
          gradient.disconnect();
          gradient.pause();
        }
      } catch {
        // Cleanup is best-effort.
      }
    };
  }, []);

  return (
    <canvas
      ref={canvasRef}
      id="mesh-gradient"
      data-transition-in
      className="absolute inset-0 w-full h-full opacity-10"
      style={
        {
          '--gradient-color-1': '#0019d9',
          '--gradient-color-2': '#b5d5ff', // primary-50
          '--gradient-color-3': '#ffffff', // primary-100
          '--gradient-color-4': '#4fa4ff', // primary-200
        } as React.CSSProperties
      }
    />
  );
}
