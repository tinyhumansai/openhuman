'use client';

import * as THREE from 'three';
import { useEffect, useRef, useState } from 'react';
import { ConvexGeometry } from 'three/addons/geometries/ConvexGeometry.js';

/** Start from a regular tetrahedron and lightly truncate each corner to create small blunted edges. */
function bluntedTetrahedronPoints(scale: number, bluntness = 0.12): THREE.Vector3[] {
  const tetra = [
    new THREE.Vector3(1, 1, 1),
    new THREE.Vector3(-1, -1, 1),
    new THREE.Vector3(-1, 1, -1),
    new THREE.Vector3(1, -1, -1),
  ];

  const points: THREE.Vector3[] = [];

  for (let i = 0; i < tetra.length; i += 1) {
    for (let j = 0; j < tetra.length; j += 1) {
      if (i === j) continue;
      points.push(tetra[i].clone().lerp(tetra[j], bluntness).multiplyScalar(scale));
    }
  }

  return points;
}

export default function RotatingTetrahedronCanvas() {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const [webglFailed, setWebglFailed] = useState(false);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    // Verify a WebGL context can be obtained before handing the canvas to
    // Three.js.  `THREE.WebGLRenderer` internally calls `gl.createShader()`
    // which throws if the context is null (e.g. when another canvas already
    // consumed the platform's WebGL context limit).
    const testCtx =
      canvas.getContext('webgl2', { antialias: true }) ||
      canvas.getContext('webgl', { antialias: true });
    if (!testCtx) {
      console.warn('[RotatingTetrahedronCanvas] WebGL context unavailable — skipping');
      setWebglFailed(true);
      return;
    }

    // Lose the test context so Three.js can create its own on the same canvas.
    // getContext returns the same context when called with the same type, so
    // Three.js will reuse it.  We just needed the null-check above.

    let renderer: THREE.WebGLRenderer;
    try {
      renderer = new THREE.WebGLRenderer({
        canvas,
        context: testCtx as WebGLRenderingContext,
        antialias: true,
        alpha: true,
        powerPreference: 'high-performance',
      });
    } catch (err) {
      console.warn('[RotatingTetrahedronCanvas] WebGLRenderer init failed:', err);
      setWebglFailed(true);
      return;
    }

    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    renderer.toneMapping = THREE.NoToneMapping;

    const scene = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera(45, 1, 0.1, 100);
    camera.position.set(0, 0.15, 4.8);

    const geometry = new ConvexGeometry(bluntedTetrahedronPoints(0.98, 0.11));
    const fillMaterial = new THREE.MeshLambertMaterial({
      color: '#8e86e9',
      transparent: true,
      opacity: 0.2,
      emissive: '#0c1208',
      emissiveIntensity: 1,
    });
    const fillMesh = new THREE.Mesh(geometry, fillMaterial);

    const edgeGeometry = new THREE.EdgesGeometry(geometry);
    const edgeMaterial = new THREE.LineBasicMaterial({ color: '#868ee9' });

    const edges = new THREE.LineSegments(edgeGeometry, edgeMaterial);
    fillMesh.rotation.x = 0.35;
    fillMesh.rotation.y = -0.15;
    edges.rotation.x = 0.35;
    edges.rotation.y = -0.15;
    scene.add(fillMesh);
    scene.add(edges);

    const ambientLight = new THREE.AmbientLight('#ffffff', 0.1);
    const sun = new THREE.DirectionalLight('#ffffff', 1.1);
    sun.position.set(4.5, 6, 5);
    scene.add(ambientLight);
    scene.add(sun);

    let animationFrame = 0;

    const resize = () => {
      const parent = canvas.parentElement;
      if (!parent) return;

      const { width, height } = parent.getBoundingClientRect();
      if (!width || !height) return;

      renderer.setSize(width, height, false);
      camera.aspect = width / height;
      camera.updateProjectionMatrix();
    };

    const observer = new ResizeObserver(resize);
    if (canvas.parentElement) observer.observe(canvas.parentElement);
    resize();

    const speed = 2;
    const animate = () => {
      fillMesh.rotation.y += 0.0038 * speed;
      fillMesh.rotation.x += 0.0002 * speed;
      edges.rotation.y += 0.0038 * speed;
      edges.rotation.x += 0.0002 * speed;

      renderer.render(scene, camera);
      animationFrame = window.requestAnimationFrame(animate);
    };

    animate();

    return () => {
      window.cancelAnimationFrame(animationFrame);
      observer.disconnect();
      edgeGeometry.dispose();
      geometry.dispose();
      fillMaterial.dispose();
      edgeMaterial.dispose();
      renderer.dispose();
    };
  }, []);

  if (webglFailed) {
    return null;
  }

  return (
    <canvas
      ref={canvasRef}
      style={{ width: '100%', height: '100%', display: 'block' }}
      aria-label="Rotating inverted tetrahedron spacecraft"
    />
  );
}
