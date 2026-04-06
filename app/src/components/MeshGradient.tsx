import { useEffect, useRef } from 'react';

import { Gradient } from '../lib/meshGradient';

/**
 * Animated WebGL mesh gradient background (Stripe-style).
 * Renders behind the dotted-canvas overlay so dots remain visible on top.
 */
export default function MeshGradient() {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    const gradient = new Gradient();
    gradient.initGradient('#mesh-gradient');

    return () => {
      gradient.disconnect();
      gradient.pause();
      if (canvas) {
        const gl = canvas.getContext('webgl') || canvas.getContext('webgl2');
        if (gl) {
          gl.getExtension('WEBGL_lose_context')?.loseContext();
        }
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
