import React from "react";

// Big pulsing red dot that replaces the face when Ghosty is recording.
// Centered on the face area (cx=520, cy=545 in the body's local viewBox).
export const RecordingFace: React.FC<{
  frame: number;
  fps: number;
  color: string;
}> = ({ frame, fps, color }) => {
  // Smooth pulse: 0..1..0 over ~1.4s.
  const t = (frame / fps) * Math.PI * (2 / 1.4);
  const pulse = 0.5 + 0.5 * Math.sin(t);

  const baseR = 190;
  const dotR = baseR + pulse * 10;

  return (
    <g>
      {/* Outer glow halo — expands and fades as the pulse rises. */}
      <circle
        cx={520}
        cy={555}
        r={baseR + 20 + pulse * 110}
        fill={color}
        opacity={0.22 * (1 - pulse)}
      />
      <circle
        cx={520}
        cy={555}
        r={baseR + 10 + pulse * 55}
        fill={color}
        opacity={0.35 * (1 - pulse * 0.8)}
      />

      {/* Solid red dot. */}
      <circle cx={520} cy={555} r={dotR} fill={color} />

      {/* Specular highlight. */}
      <ellipse
        cx={465}
        cy={495}
        rx={50}
        ry={28}
        fill="#ffffff"
        opacity={0.22}
      />
    </g>
  );
};
