import React from "react";
import { AbsoluteFill, interpolate, useCurrentFrame, useVideoConfig } from "remotion";
import { GhostyCharacter, ghostySchema, type GhostyProps } from "./lib";

// Variant: Ghosty being "picked up" — anticipation squash, lift into the air,
// hover, drop back down, settle. The animation loops cleanly back to the start.
export const ghostyPickupSchema = ghostySchema;
export type GhostyPickupProps = GhostyProps;

export const GhostyPickup: React.FC<GhostyPickupProps> = (props) => {
  const frame = useCurrentFrame();
  const { fps, height } = useVideoConfig();
  const t = frame / fps; // seconds

  // Keyframe times (seconds): start → anticipation → launch → hover → land → settle.
  const times = [0.0, 0.35, 0.55, 1.2, 2.6, 3.0, 3.6, 4.0];

  // Vertical lift (negative = up). Expressed as a fraction of the canvas height
  // so it scales with composition size.
  const liftFrac = interpolate(
    t,
    times,
    [0, 0, -0.04, -0.32, -0.32, -0.04, 0, 0],
    { extrapolateLeft: "clamp", extrapolateRight: "clamp" },
  );
  const lift = liftFrac * height;

  // Squash: anticipation (wider/shorter), launch (taller/thinner), hover (neutral),
  // landing (wider/shorter again), settle (neutral).
  const sx = interpolate(
    t,
    times,
    [1, 1.08, 0.96, 1, 1, 1.07, 1, 1],
    { extrapolateLeft: "clamp", extrapolateRight: "clamp" },
  );
  const sy = interpolate(
    t,
    times,
    [1, 0.88, 1.05, 1, 1, 0.9, 1, 1],
    { extrapolateLeft: "clamp", extrapolateRight: "clamp" },
  );

  // Slight sway during the hover phase.
  const inHover = t >= 1.2 && t <= 2.6;
  const sway = inHover ? Math.sin((t - 1.2) * Math.PI * 1.6) * 6 : 0;

  return (
    <AbsoluteFill
      style={{
        // Squash anchored at the base of the canvas so the feet feel grounded.
        transformOrigin: "50% 100%",
        transform: `translate(${sway}px, ${lift}px) scale(${sx}, ${sy})`,
      }}
    >
      <GhostyCharacter
        {...props}
        arm={props.arm ?? "none"}
        face={props.face ?? "normal"}
        idPrefix="ghosty-pickup"
      />
    </AbsoluteFill>
  );
};
