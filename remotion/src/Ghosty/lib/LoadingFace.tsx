import React from "react";

// Spinning circular loading indicator that replaces the face.
// Centered on the face area (cx=520, cy=545 in the body's local viewBox).
export const LoadingFace: React.FC<{
  frame: number;
  fps: number;
  color: string;
  trackColor?: string;
}> = ({ frame, fps, color, trackColor = "#ffffff" }) => {
  // One full rotation every 1.4 seconds.
  const rotation = ((frame / fps) * 360) / 1.4;

  const radius = 175;
  const stroke = 28;
  const circumference = 2 * Math.PI * radius;
  // The visible arc occupies ~70% of the circumference; the rest is the gap that spins.
  const arc = circumference * 0.7;

  return (
    <g transform={`translate(520 555)`}>
      {/* Background track. */}
      <circle
        cx={0}
        cy={0}
        r={radius}
        fill="none"
        stroke={trackColor}
        strokeOpacity={0.18}
        strokeWidth={stroke}
      />

      {/* Spinning progress arc. */}
      <g transform={`rotate(${rotation})`}>
        <circle
          cx={0}
          cy={0}
          r={radius}
          fill="none"
          stroke={color}
          strokeWidth={stroke}
          strokeLinecap="round"
          strokeDasharray={`${arc} ${circumference - arc}`}
          strokeDashoffset={0}
        />
      </g>
    </g>
  );
};
