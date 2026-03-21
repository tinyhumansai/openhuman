'use client';

import { useEffect, useRef } from 'react';
import * as THREE from 'three';
import { ConvexGeometry } from 'three/addons/geometries/ConvexGeometry.js';

/** Canonical vertices of a truncated tetrahedron (permutations of 0, ±1, ±1). Convex hull = Archimedean solid. */
function truncatedTetrahedronPoints(scale: number): THREE.Vector3[] {
  const coords: [number, number, number][] = [
    [0, 1, 1],
    [0, 1, -1],
    [0, -1, 1],
    [0, -1, -1],
    [1, 0, 1],
    [1, 0, -1],
    [-1, 0, 1],
    [-1, 0, -1],
    [1, 1, 0],
    [1, -1, 0],
    [-1, 1, 0],
    [-1, -1, 0],
  ];
  return coords.map(([x, y, z]) => new THREE.Vector3(x, y, z).multiplyScalar(scale));
}

export default function RotatingTetrahedronCanvas() {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const renderer = new THREE.WebGLRenderer({
      canvas,
      antialias: true,
      alpha: true,
      powerPreference: 'high-performance',
    });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    renderer.toneMapping = THREE.NoToneMapping;

    const scene = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera(45, 1, 0.1, 100);
    camera.position.set(0, 0.15, 4.8);

    const geometry = new ConvexGeometry(truncatedTetrahedronPoints(0.92));

    // Flat, panel-like read: matte off-white / reads as light vs shadow (Oblivion-style blocking).
    const fillMaterial = new THREE.MeshLambertMaterial({
      color: '#e4e4e4',
      emissive: '#080808',
      emissiveIntensity: 0.06,
    });

    const fillMesh = new THREE.Mesh(geometry, fillMaterial);
    fillMesh.rotation.x = 0.35;
    fillMesh.rotation.y = -0.45;
    scene.add(fillMesh);

    const ambientLight = new THREE.AmbientLight('#ffffff', 0.08);
    const sun = new THREE.DirectionalLight('#ffffff', 1.35);
    sun.position.set(4.5, 6, 5);
    const bounce = new THREE.DirectionalLight('#8899aa', 0.22);
    bounce.position.set(-3, -1, -2);

    scene.add(ambientLight);
    scene.add(sun);
    scene.add(bounce);

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

    const animate = () => {
      fillMesh.rotation.y += 0.007;
      fillMesh.rotation.x += 0.0045;

      renderer.render(scene, camera);
      animationFrame = window.requestAnimationFrame(animate);
    };

    animate();

    return () => {
      window.cancelAnimationFrame(animationFrame);
      observer.disconnect();
      geometry.dispose();
      fillMaterial.dispose();
      renderer.dispose();
    };
  }, []);

  return (
    <canvas ref={canvasRef} className="h-full w-full block" aria-label="Rotating tetrahedron" />
  );
}
