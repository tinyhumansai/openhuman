import React from "react";
import { AbsoluteFill, interpolate, useCurrentFrame, useVideoConfig } from "remotion";
import { MascotCharacter, mascotSchema, type MascotProps } from "./lib";

// Variant: simple bouncy squash-and-stretch in place.
export const yellowMascotPickupSchema = mascotSchema;
export type YellowMascotPickupProps = MascotProps;

export const YellowMascotPickup: React.FC<YellowMascotPickupProps> = (props) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const t = frame / fps;

  // Three bounces with decreasing squash + a small upward hop each peak.
  const times = [0, 0.18, 0.36, 0.54, 0.72, 0.90, 1.08, 4.0];
  const sx = interpolate(t, times, [1, 1.18, 1, 1.12, 1, 1.06, 1, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  const sy = interpolate(t, times, [1, 0.74, 1, 0.82, 1, 0.91, 1, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  // Slight upward hop at each bounce peak (negative = up). Max 40 px.
  const ly = interpolate(t, times, [0, 0, -90, 0, -50, 0, -20, 0], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });

  return (
    <AbsoluteFill
      style={{
        transformOrigin: "50% 100%",
        transform: `translateY(${ly}px) scale(${sx}, ${sy})`,
      }}
    >
      <MascotCharacter
        {...props}
        arm={props.arm ?? "none"}
        face={props.face ?? "normal"}
        idPrefix="mascot-pickup"
      />
    </AbsoluteFill>
  );
};
