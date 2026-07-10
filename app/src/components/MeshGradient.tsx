import { useEffect, useRef } from 'react';

import { Gradient } from '../lib/meshGradient';
import { channelsToHex } from '../lib/theme/color';
import { useAppSelector } from '../store/hooks';
import { selectEffectiveTheme, selectThemeVariant } from '../store/themeSlice';

/**
 * Animated WebGL mesh gradient background (Stripe-style), tinted by the active
 * theme. The four gradient stops are derived from the theme's primary ramp +
 * surface token (read as hex from the resolved CSS variables), so the backdrop
 * follows Matrix green / HAL red / Ocean blue / etc. It re-initialises whenever
 * the active theme or its light/dark variant changes.
 *
 * Renders behind the dotted-canvas overlay so dots remain visible on top, and
 * catches WebGL errors gracefully (Tauri WebView can lack a GPU context).
 */
export default function MeshGradient() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  // Re-init the gradient when the effective theme's mesh-driving tokens change.
  const effectiveTheme = useAppSelector(selectEffectiveTheme);
  const variant = useAppSelector(selectThemeVariant);
  const tokenSignature = [
    effectiveTheme.id,
    effectiveTheme.isDark ? 'dark' : 'light',
    effectiveTheme.colors['primary-700'] ?? '',
    effectiveTheme.colors['primary-300'] ?? '',
    effectiveTheme.colors.surface ?? '',
    effectiveTheme.colors['primary-500'] ?? '',
  ].join('|');

  useEffect(() => {
    let gradient: InstanceType<typeof Gradient> | null = null;
    let raf = 0;
    let disposed = false;

    const disconnectGradient = () => {
      try {
        if (gradient) {
          gradient.disconnect();
          gradient.pause();
        }
      } catch {
        // Cleanup is best-effort.
      } finally {
        gradient = null;
      }
    };

    const start = () => {
      if (disposed) return;
      const root = document.documentElement;
      const canvas = canvasRef.current;
      if (!canvas) return;
      disconnectGradient();
      const hex = (token: string, fallback: string) => {
        const v = window.getComputedStyle(root).getPropertyValue(`--${token}`).trim();
        return v ? channelsToHex(v) : fallback;
      };
      // Accent-led stops + the surface base, so the mesh reads on any theme.
      canvas.style.setProperty('--gradient-color-1', hex('primary-700', '#0019d9'));
      canvas.style.setProperty('--gradient-color-2', hex('primary-300', '#b5d5ff'));
      canvas.style.setProperty('--gradient-color-3', hex('surface', '#ffffff'));
      canvas.style.setProperty('--gradient-color-4', hex('primary-500', '#4fa4ff'));
      console.debug('[theme] mesh gradient stops applied', { themeId: effectiveTheme.id, variant });

      try {
        gradient = new Gradient();
        gradient.initGradient('#mesh-gradient');
      } catch (err) {
        console.warn('[MeshGradient] WebGL init failed, gradient disabled:', err);
        gradient = null;
      }
    };

    const scheduleStart = () => {
      window.cancelAnimationFrame(raf);
      // Defer one frame so ThemeProvider (an ancestor) has applied the theme's
      // CSS variables before we read them — child effects run before parents.
      raf = window.requestAnimationFrame(start);
    };

    scheduleStart();

    let removeSystemListener: (() => void) | undefined;
    if (variant === 'system' && typeof window !== 'undefined' && window.matchMedia) {
      const mq = window.matchMedia('(prefers-color-scheme: dark)');
      const listener = () => scheduleStart();
      if (mq.addEventListener) {
        mq.addEventListener('change', listener);
        removeSystemListener = () => mq.removeEventListener('change', listener);
      } else {
        mq.addListener(listener);
        removeSystemListener = () => mq.removeListener(listener);
      }
    }

    return () => {
      disposed = true;
      removeSystemListener?.();
      window.cancelAnimationFrame(raf);
      disconnectGradient();
    };
  }, [effectiveTheme.id, tokenSignature, variant]);

  return (
    <canvas
      ref={canvasRef}
      id="mesh-gradient"
      data-transition-in
      className="absolute inset-0 w-full h-full opacity-10"
    />
  );
}
