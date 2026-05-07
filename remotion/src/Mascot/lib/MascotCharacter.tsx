import React from "react";
import { AbsoluteFill, Easing, interpolate, useCurrentFrame, useVideoConfig } from "remotion";
import { z } from "zod";
import { zColor } from "@remotion/zod-types";
import { RecordingFace } from "../../Ghosty/lib/RecordingFace";
import { LoadingFace } from "../../Ghosty/lib/LoadingFace";
import { getMascotPalette, type MascotColor } from "./mascotPalette";

export const mascotSchema = z.object({
  arm: z.enum(["wave", "none", "steady"]).default("wave"),
  face: z.enum(["normal", "recording", "loading"]).default("normal"),
  talking: z.boolean().default(false),
  sleeping: z.boolean().default(false),
  thinking: z.boolean().default(false),
  greeting: z.boolean().default(false),
  mascotColor: z.enum(["yellow", "burgundy", "black", "navy", "green"]).default("yellow"),
  recordingColor: zColor().default("#ff3b30"),
  loadingColor: zColor().default("#ffffff"),
});

export type MascotProps = z.infer<typeof mascotSchema>;

/**
 * Mascot character — drives the custom yellow mascot SVG with the same
 * animation system as Ghosty: body bob, head-dot drift/squash, arm wave, blink.
 *
 * Use distinct `idPrefix` values if two instances appear in the same SVG tree
 * so filter/gradient IDs don't collide.
 */
type ThinkingTiming = {
  /** Seconds at which the idle→thinking ramp begins. Default 1.0. */
  thinkInStartSec?: number;
  /** Seconds at which the idle→thinking ramp completes. Default 2.0. */
  thinkInEndSec?: number;
  /** Seconds at which the thinking→idle ramp begins. If unset, the pose holds. */
  thinkOutStartSec?: number;
  /** Seconds at which the thinking→idle ramp completes. Required if thinkOutStartSec is set. */
  thinkOutEndSec?: number;
};

export const MascotCharacter: React.FC<MascotProps & { idPrefix?: string } & ThinkingTiming> = ({
  arm = "wave",
  face = "normal",
  talking = false,
  sleeping = false,
  thinking = false,
  greeting = false,
  mascotColor = "yellow",
  recordingColor = "#ff3b30",
  loadingColor = "#ffffff",
  idPrefix = "mascot",
  thinkInStartSec = 1.0,
  thinkInEndSec = 2.0,
  thinkOutStartSec,
  thinkOutEndSec,
}) => {
  const palette = getMascotPalette(mascotColor as MascotColor);
  const frame = useCurrentFrame();
  const { fps, width, height } = useVideoConfig();

  // Gentle bob for the whole character.
  const bob = Math.sin((frame / fps) * Math.PI * 1.2) * 14;

  // Head dot drifts independently and squashes when pressing into the body.
  const dotPhase = (frame / fps) * Math.PI * 1.0;
  const dotDx = Math.sin(dotPhase * 0.7) * 6;
  const dotDy = Math.sin(dotPhase) * 9;
  const press = Math.max(0, Math.sin(dotPhase));
  const dotSquashY = 1 - 0.08 * press;
  const dotSquashX = 1 + 0.05 * press;

  // Right arm wave — keyframe-based hi-wave: 3 swings then a rest pause, loops every 2.4s.
  // Negative rotation = arm tips upward (counterclockwise). Eased for natural feel.
  const easeInOut = Easing.inOut(Easing.cubic);
  const wavePeriod = Math.round(fps * 2.4);
  const frameInCycle = frame % wavePeriod;
  const wave = arm === "wave"
    ? interpolate(
        frameInCycle,
        [0, wavePeriod * 0.12, wavePeriod * 0.25, wavePeriod * 0.38, wavePeriod * 0.50, wavePeriod * 0.62, wavePeriod * 0.75, wavePeriod],
        [0, -9, 0, -7, 0, -5, 0, 0],
        { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: easeInOut },
      )
    : 0;

  // Left arm gentle sway — slower frequency, smaller amplitude.
  const leftSway = Math.sin((frame / fps) * Math.PI * 1.6) * 7;

  // Steady right arm sway — mirrors left arm with slight phase offset.
  const steadySway = Math.sin((frame / fps) * Math.PI * 1.6 + 0.3) * 6;

  // Lip sync — slowed to ~1.5–2.3 Hz for natural speech pace (was 2.25–3.55 Hz).
  // Phase offset keeps them from closing simultaneously.
  const talkA = Math.abs(Math.sin((frame / fps) * Math.PI * 3.0));
  const talkB = Math.abs(Math.sin((frame / fps) * Math.PI * 4.6 + 1.2));
  const mouthOpen = talking ? Math.max(talkA, talkB * 0.8) : 0;
  // Tongue fades in only when mouth is open enough — prevents visible tongue during near-closed frames.
  const tongueOpacity = talking ? Math.min(1, Math.max(0, (mouthOpen - 0.15) / 0.35)) : 0;

  // Blink every ~2.6s for ~6 frames.
  const blinkPeriod = Math.round(fps * 2.6);
  const blinkOffset = Math.round(blinkPeriod / 2);
  const inBlink = (frame + blinkOffset) % blinkPeriod < 6;
  const blinkScale = inBlink ? 0.12 : 1;

  // Sleep animation — slow eye-close then floating Zzz.
  const sleepStartFrame = sleeping ? Math.round(fps * 2.5) : 99999;
  const sleepFullFrame  = sleeping ? Math.round(fps * 4.0) : 99999;
  const inSleepTransition = sleeping && frame >= sleepStartFrame;
  const sleepProgress = sleeping
    ? interpolate(frame, [sleepStartFrame, sleepFullFrame], [0, 1], {
        extrapolateLeft: "clamp",
        extrapolateRight: "clamp",
        easing: Easing.inOut(Easing.cubic),
      })
    : 0;
  const isAsleep = sleeping && frame >= sleepFullFrame;

  // Eye openness: normal blink while awake, slow droop during sleep transition.
  const eyeScale = inSleepTransition ? Math.max(0, 1 - sleepProgress) : blinkScale;
  // Suppress blink highlights mid-droop so pupils don't pop on/off.
  const effectiveInBlink = inSleepTransition ? false : inBlink;
  // Switch to sleep-arc eyes once eyelids have closed.
  const showSleepEyes = sleeping && eyeScale <= 0.06;

  // Floating Z letters — staggered, drift up and fade out.
  const zPeriod   = Math.round(fps * 2.2);
  const zBaseStart = sleepFullFrame + Math.round(fps * 0.4);
  const getZ = (delay: number, baseX: number, fontSize: number) => {
    const startAt = zBaseStart + delay;
    if (!isAsleep || frame < startAt) return { x: baseX, y: 220 as number, opacity: 0 as number, fontSize };
    const cycleFrame = (frame - startAt) % zPeriod;
    const t = cycleFrame / zPeriod;
    return {
      x: baseX + t * 20,
      y: 220 - t * 120,
      opacity: interpolate(t, [0, 0.1, 0.72, 1], [0, 1, 0.85, 0]),
      fontSize,
    };
  };
  // Thinking animation — arm raises, head tilts, eyes shift up, mouth changes.
  // Ramp up from `thinkInStartSec` → `thinkInEndSec`. If thinkOutStartSec/EndSec
  // are provided, ramp back down so the pose returns to idle (loop-friendly).
  const thinkStartFrame = thinking ? Math.round(fps * thinkInStartSec) : 99999;
  const thinkFullFrame  = thinking ? Math.round(fps * thinkInEndSec)   : 99999;
  const hasOutRamp = thinking && thinkOutStartSec !== undefined && thinkOutEndSec !== undefined;
  const thinkOutStartFrame = hasOutRamp ? Math.round(fps * (thinkOutStartSec as number)) : 99999;
  const thinkOutEndFrame   = hasOutRamp ? Math.round(fps * (thinkOutEndSec as number))   : 99999;
  const thinkInProgress = thinking
    ? interpolate(frame, [thinkStartFrame, thinkFullFrame], [0, 1], {
        extrapolateLeft: "clamp",
        extrapolateRight: "clamp",
        easing: Easing.inOut(Easing.cubic),
      })
    : 0;
  const thinkOutProgress = hasOutRamp
    ? interpolate(frame, [thinkOutStartFrame, thinkOutEndFrame], [0, 1], {
        extrapolateLeft: "clamp",
        extrapolateRight: "clamp",
        easing: Easing.inOut(Easing.cubic),
      })
    : 0;
  const thinkProgress = Math.max(0, thinkInProgress - thinkOutProgress);
  // "Fully in pose" — only true while held between in-ramp end and out-ramp start.
  const isThinking = thinking && thinkInProgress >= 1 && thinkOutProgress <= 0;

  // LEFT arm raises toward body/chin for thinking pose (matches reference: arm on viewer's left side).
  // Normal left arm droops at ~127° from +x axis; rotating −128° brings it to ~−1°
  // (nearly horizontal, pointing right toward body center — "hand near chin" read).
  const thinkArmOscillate = isThinking ? Math.sin((frame / fps) * Math.PI * 0.5) * 2 : 0;
  const effectiveLeftSway = thinking
    ? interpolate(thinkProgress, [0, 1], [leftSway, -128]) + thinkArmOscillate
    : leftSway;

  // Right arm stays in normal steady position while thinking.
  const rightSteadyAngle = steadySway;

  // Head tilts slightly toward raised arm (left = negative rotation in SVG).
  const headTilt = isThinking
    ? -4.5 + Math.sin((frame / fps) * Math.PI * 0.38) * 1.8
    : thinking
      ? interpolate(thinkProgress, [0, 1], [0, -4.5])
      : 0;

  // Eyes drift up-left — looking toward the raised arm / into the distance.
  const thinkEyeX = thinking ? thinkProgress * -6 : 0;
  const thinkEyeY = thinking ? thinkProgress * -9 : 0;

  // Greeting — right arm rises from resting to raised, then waves "hi" in a loop.
  const greetStartFrame = greeting ? Math.round(fps * 0.8) : 99999;
  const greetRaiseEnd   = greeting ? Math.round(fps * 1.6) : 99999;
  const isGreeting = greeting && frame >= greetStartFrame;
  const greetRaiseProgress = greeting
    ? interpolate(frame, [greetStartFrame, greetRaiseEnd], [0, 1], {
        extrapolateLeft: "clamp",
        extrapolateRight: "clamp",
        easing: Easing.out(Easing.cubic),
      })
    : 0;
  // Raise: wave arm rotates from +52° (arm pointing right/down) up to 0° (arm raised).
  const greetRaiseAngle = interpolate(greetRaiseProgress, [0, 1], [52, 0]);
  // Hi wave: enthusiastic oscillation after the arm is fully raised.
  const greetWavePeriod = Math.round(fps * 1.3);
  const greetWaveFrame  = (greeting && frame > greetRaiseEnd)
    ? (frame - greetRaiseEnd) % greetWavePeriod
    : 0;
  const greetWaveOscillate = (greeting && frame > greetRaiseEnd)
    ? interpolate(
        greetWaveFrame,
        [0, greetWavePeriod * 0.25, greetWavePeriod * 0.5, greetWavePeriod * 0.75, greetWavePeriod],
        [0, -28, -2, -26, 0],
        { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: Easing.inOut(Easing.cubic) },
      )
    : 0;
  const greetArmAngle = greetRaiseAngle + greetWaveOscillate;

  const z1 = getZ(0,                         605, 40);
  const z2 = getZ(Math.round(fps * 0.72),    624, 56);
  const z3 = getZ(Math.round(fps * 1.44),    643, 76);

  const size = Math.min(width, height) * 0.85;
  const p = (k: string) => `${idPrefix}-${k}`;

  return (
    <AbsoluteFill style={{ justifyContent: "center", alignItems: "center" }}>
      <svg
        width={size}
        height={size}
        viewBox="0 0 1000 1000"
        style={{ overflow: "visible" }}
      >
        <defs>
          {/* Ground shadow gradient */}
          <radialGradient id={p("ground")} cx="0.5" cy="0.5" r="0.5">
            <stop offset="0%" stopColor="#000000" stopOpacity="0.35" />
            <stop offset="100%" stopColor="#000000" stopOpacity="0" />
          </radialGradient>

          {/* filter0: body — inner shadows + grain texture */}
          <filter id={p("f0")} x="90.3857" y="238.634" width="765.268" height="762.131" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha" />
            <feOffset dx="17" dy="28" />
            <feGaussianBlur stdDeviation="10.45" />
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1" />
            <feColorMatrix type="matrix" values={palette.bodyHighlightMatrix} />
            <feBlend mode="normal" in2="shape" result="effect1_innerShadow" />
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha" />
            <feOffset dx="-27" dy="-22" />
            <feGaussianBlur stdDeviation="29.75" />
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1" />
            <feColorMatrix type="matrix" values={palette.bodyShadowMatrix} />
            <feBlend mode="normal" in2="effect1_innerShadow" result="effect2_innerShadow" />
            <feTurbulence type="fractalNoise" baseFrequency="0.999" numOctaves={3} seed={8703} />
            <feDisplacementMap in="effect2_innerShadow" scale={8} xChannelSelector="R" yChannelSelector="G" result="displacedImage" width="100%" height="100%" />
            <feMerge><feMergeNode in="displacedImage" /></feMerge>
          </filter>

          {/* filter1: head circle — inner shadows + grain texture */}
          <filter id={p("f1")} x="379" y="22" width="233" height="237" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha" />
            <feOffset dx="9" dy="2" />
            <feGaussianBlur stdDeviation="5.65" />
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1" />
            <feColorMatrix type="matrix" values={palette.headHighlightMatrix} />
            <feBlend mode="normal" in2="shape" result="effect1_innerShadow" />
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha" />
            <feOffset dx="-2" dy="-13" />
            <feGaussianBlur stdDeviation="19.7" />
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1" />
            <feColorMatrix type="matrix" values={palette.headShadowMatrix} />
            <feBlend mode="normal" in2="effect1_innerShadow" result="effect2_innerShadow" />
            <feTurbulence type="fractalNoise" baseFrequency="0.999" numOctaves={3} seed={8703} />
            <feDisplacementMap in="effect2_innerShadow" scale={8} xChannelSelector="R" yChannelSelector="G" result="displacedImage" width="100%" height="100%" />
            <feMerge><feMergeNode in="displacedImage" /></feMerge>
          </filter>

          {/* filter2: neck shadow 1 — blur */}
          <filter id={p("f2")} x="423.5" y="239.5" width="153.771" height="66.8604" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feGaussianBlur stdDeviation="11.25" />
          </filter>

          {/* filter3: neck shadow 2 — blur */}
          <filter id={p("f3")} x="434.976" y="217.946" width="123.537" height="57.3711" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feGaussianBlur stdDeviation="11.25" />
          </filter>

          {/* filter4: right arm — inner shadows + grain texture */}
          <filter id={p("f4")} x="759.925" y="474.413" width="170.767" height="268.758" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha" />
            <feOffset dx="-11" dy="28" />
            <feGaussianBlur stdDeviation="11" />
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1" />
            <feColorMatrix type="matrix" values={palette.armHighlightMatrix} />
            <feBlend mode="normal" in2="shape" result="effect1_innerShadow" />
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha" />
            <feOffset dx="-8" dy="1" />
            <feGaussianBlur stdDeviation="4.25" />
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1" />
            <feColorMatrix type="matrix" values={palette.armShadowMatrix} />
            <feBlend mode="normal" in2="effect1_innerShadow" result="effect2_innerShadow" />
            <feTurbulence type="fractalNoise" baseFrequency="0.999" numOctaves={3} seed={8703} />
            <feDisplacementMap in="effect2_innerShadow" scale={8} xChannelSelector="R" yChannelSelector="G" result="displacedImage" width="100%" height="100%" />
            <feMerge><feMergeNode in="displacedImage" /></feMerge>
          </filter>

          {/* filter5: left arm — inner shadows + grain texture */}
          <filter id={p("f5")} x="138.458" y="555.812" width="155.093" height="272.386" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha" />
            <feOffset dx="1" dy="-20" />
            <feGaussianBlur stdDeviation="7.55" />
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1" />
            <feColorMatrix type="matrix" values={palette.armHighlightMatrix} />
            <feBlend mode="normal" in2="shape" result="effect1_innerShadow" />
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha" />
            <feOffset dx="3" dy="-8" />
            <feGaussianBlur stdDeviation="3.55" />
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1" />
            <feColorMatrix type="matrix" values={palette.armShadowMatrix.replace(/ 1 0$/, " 0.8 0")} />
            <feBlend mode="normal" in2="effect1_innerShadow" result="effect2_innerShadow" />
            <feTurbulence type="fractalNoise" baseFrequency="0.999" numOctaves={3} seed={8703} />
            <feDisplacementMap in="effect2_innerShadow" scale={8} xChannelSelector="R" yChannelSelector="G" result="displacedImage" width="100%" height="100%" />
            <feMerge><feMergeNode in="displacedImage" /></feMerge>
          </filter>

          {/* filter6-7: left eye highlights */}
          <filter id={p("f6")} x="390.218" y="433.891" width="25.0341" height="28.893" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feGaussianBlur stdDeviation="0.65" />
          </filter>
          <filter id={p("f7")} x="390.3" y="434.3" width="22.4" height="23.4" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feGaussianBlur stdDeviation="0.35" />
          </filter>

          {/* filter8-10: right eye highlights */}
          <filter id={p("f8")} x="570.859" y="435.358" width="27.0393" height="29.1125" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feGaussianBlur stdDeviation="0.95" />
          </filter>
          <filter id={p("f9")} x="571.3" y="436.3" width="19.4" height="20.4" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feGaussianBlur stdDeviation="0.35" />
          </filter>
          <filter id={p("f10")} x="574.668" y="440.492" width="10.9674" height="13.0943" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feGaussianBlur stdDeviation="0.35" />
          </filter>

          {/* filter13: steady right arm (idle pose) — mirrors left arm, inner shadows + grain */}
          <filter id={p("f13")} x="645" y="555.9" width="155.094" height="272.386" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha" />
            <feOffset dx="1" dy="-20" />
            <feGaussianBlur stdDeviation="7.55" />
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1" />
            <feColorMatrix type="matrix" values={palette.armHighlightMatrix} />
            <feBlend mode="normal" in2="shape" result="effect1_innerShadow" />
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha" />
            <feOffset dx="0" dy="-8" />
            <feGaussianBlur stdDeviation="3.55" />
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1" />
            <feColorMatrix type="matrix" values={palette.armShadowMatrix.replace(/ 1 0$/, " 0.8 0")} />
            <feBlend mode="normal" in2="effect1_innerShadow" result="effect2_innerShadow" />
            <feTurbulence type="fractalNoise" baseFrequency="0.999" numOctaves={3} seed={8703} />
            <feDisplacementMap in="effect2_innerShadow" scale={8} xChannelSelector="R" yChannelSelector="G" result="displacedImage" width="100%" height="100%" />
            <feMerge><feMergeNode in="displacedImage" /></feMerge>
          </filter>

          {/* filter11-12: cheek highlights */}
          <filter id={p("f11")} x="366.181" y="492.2" width="15.6322" height="13.601" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feGaussianBlur stdDeviation="0.9" />
          </filter>
          <filter id={p("f12")} x="618.2" y="495.2" width="15.6322" height="13.601" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feGaussianBlur stdDeviation="0.9" />
          </filter>
        </defs>

        {/* Ground shadow — scales with bob so it feels grounded. */}
        <g transform={`translate(500, 975) scale(${1 - bob / 600}, 1)`}>
          <ellipse cx={0} cy={0} rx={300} ry={28} fill={`url(#${p("ground")})`} />
        </g>

        {/* Everything bobs together. */}
        <g transform={`translate(0, ${bob})`}>

          {/* Head dot — drifts + squashes independently inside the bob group. */}
          <g transform={
            `translate(${dotDx}, ${dotDy}) ` +
            `translate(493 145) scale(${dotSquashX} ${dotSquashY}) translate(-493 -145)`
          }>
            <circle cx={493} cy={145} r={110} fill={palette.bodyFill} filter={`url(#${p("f1")})`} />
          </g>

          {/* Body */}
          <path
            d="M270.548 382.714C175.869 479.647 86.1402 654.573 127.915 829.517C145.272 881.371 165.202 911.976 222.935 941.975C253.337 957.772 327.5 950.5 375.544 921.664L445.394 890.456C490.742 873.851 509.572 876.412 538.5 889.192C577.029 910.413 587.5 931.5 649.207 964.222C729.487 1006.79 793.127 956.041 817.514 889.192C874.808 742.915 814.514 422.978 650.331 310.479C516.054 226.594 403.003 247.226 270.548 382.714Z"
            fill={palette.bodyFill}
            filter={`url(#${p("f0")})`}
          />

          {/* Waving right arm — normal wave OR greeting raise+hi-wave. */}
          {(arm === "wave" || isGreeting) && (
            <g transform={`rotate(${isGreeting ? greetArmAngle : wave}, 776, 568)`}>
              <path
                d="M821.855 513.95C798.846 545.418 795.5 553 776.706 568C760.334 581.067 781.974 653.709 801.375 710.888C805.052 721.724 819.237 724.693 827.147 716.425C860.877 681.172 917.862 621.391 924.689 572.869C939.558 467.192 868.275 454.188 821.855 513.95Z"
                fill={palette.bodyFill}
                filter={`url(#${p("f4")})`}
              />
            </g>
          )}

          {/* Steady right arm — hidden once greeting raise begins. */}
          {arm === "steady" && !isGreeting && (
            <g transform={`rotate(${rightSteadyAngle}, 655, 709)`}>
              <path
                d="M680.851 773.156C666.823 736.786 665.565 728.594 651.321 709.221C638.913 692.343 678.709 627.834 712.32 577.674C718.689 568.167 733.158 568.991 738.645 579.033C762.04 621.848 801.508 694.398 795.474 743.024C782.333 848.93 710.122 842.939 680.851 773.156Z"
                fill={palette.bodyFill}
                filter={`url(#${p("f13")})`}
              />
            </g>
          )}

          {/* Left arm — gentle sway in idle; rotates up toward body center while thinking. */}
          <g transform={`rotate(${effectiveLeftSway}, 290, 700)`}>
            <path
              d="M257.7 773.068C271.728 736.698 272.987 728.506 287.23 709.133C299.638 692.255 259.842 627.746 226.232 577.586C219.862 568.08 205.393 568.903 199.906 578.945C176.511 621.76 137.044 694.31 143.077 742.936C156.218 848.842 228.429 842.851 257.7 773.068Z"
              fill={palette.bodyFill}
              filter={`url(#${p("f5")})`}
            />
          </g>

          {/* Neck shadow details */}
          <g opacity={0.4} filter={`url(#${p("f2")})`}>
            <path d="M450.376 270.172C464.042 264.005 502.076 255.372 544.876 270.172C598.376 288.672 415.876 288.172 450.376 270.172Z" fill={palette.neckShadowColor} />
          </g>
          <g opacity={0.4} filter={`url(#${p("f3")})`}>
            <path d="M533.5 245.499C524.956 248.602 489.943 257.335 463.186 249.888C429.739 240.578 555.068 236.442 533.5 245.499Z" fill={palette.neckShadowColor} />
          </g>

          {/* Normal face — eyes, cheeks, mouth.
              Wrapped in a rotation group for the thinking head-tilt. */}
          {face === "normal" && (
            <g transform={`rotate(${headTilt}, 495, 375)`}>
              {/* Sleep eyes — curved closed-lid arcs, visible only when eyeScale ≈ 0 */}
              {showSleepEyes && (
                <>
                  <path d="M390,466 Q411,481 436,466" stroke="#1C170B" strokeWidth="5.5" strokeLinecap="round" fill="none" />
                  <path d="M563,466 Q589,481 615,466" stroke="#1C170B" strokeWidth="5.5" strokeLinecap="round" fill="none" />
                </>
              )}

              {/* Left eye — scaleY collapses on blink/sleep; translate shifts gaze while thinking */}
              {!showSleepEyes && (
              <g transform={`translate(${thinkEyeX}, ${thinkEyeY})`}>
              <g transform={`translate(411, 465) scale(1, ${eyeScale}) translate(-411, -465)`}>
                <path d="M411.48 428C419.679 428 423 432 424.408 434.321C431.456 442.807 434.448 450.812 435.286 461.939C436.531 478.451 428.581 501.025 409.176 501.922C402.907 502.212 396.783 499.978 392.177 495.714C372.967 478.168 379.456 428.811 411.48 428Z" fill="#1C170B" />
                {!effectiveInBlink && (
                  <>
                    <g filter={`url(#${p("f6")})`}>
                      <path d="M402.589 435.31C405.113 435.115 406.119 435.015 408.226 436.218C409.449 437.699 409.295 438.305 409.367 440.116C410.18 440.625 410.898 441.111 411.694 441.647L411.904 442.956C419.014 456.194 406.034 468.295 397.004 457.028C387.109 457.791 393.027 445.603 396.045 441.344C398.038 438.531 399.869 437.302 402.589 435.31Z" fill="#FAF3EC" />
                    </g>
                    <g filter={`url(#${p("f7")})`}>
                      <path d="M402.405 435.12C405.005 434.923 406.041 434.822 408.211 436.033C409.471 437.522 409.312 438.132 409.386 439.954C410.224 440.465 410.964 440.954 411.784 441.493L412 442.811C408.557 441.118 406.625 439.187 402.54 440.654C395.773 443.086 394.268 451.112 396.652 456.966C386.459 457.733 392.555 445.473 395.664 441.189C397.717 438.36 399.602 437.123 402.405 435.12Z" fill="#3A372F" />
                    </g>
                  </>
                )}
              </g>
              </g>
              )}

              {/* Right eye — same blink / sleep; translate shifts gaze while thinking */}
              {!showSleepEyes && (
              <g transform={`translate(${thinkEyeX}, ${thinkEyeY})`}>
              <g transform={`translate(589, 465) scale(1, ${eyeScale}) translate(-589, -465)`}>
                <path d="M589.37 428.706C621.867 428.523 630.994 493.598 594.352 502.663C555.686 504.419 554.456 433.119 589.37 428.706Z" fill="#1C170B" />
                {!effectiveInBlink && (
                  <>
                    <g filter={`url(#${p("f8")})`}>
                      <path d="M576.491 452.759C577.097 454.049 577.14 454.759 576.609 455.979C569.334 454.164 573.452 439.586 580.007 437.664C584.2 436.436 587.824 438.013 589.306 442.115C592.619 444.137 594.847 446.01 595.749 450.049C596.355 452.791 595.845 455.661 594.331 458.027C589.038 466.354 580.303 462.46 578.515 452.619C577.656 451.775 577.93 451.624 577.758 450.079L577.591 450.499L577.887 450.8L577.387 452.615L576.491 452.759Z" fill="#FAF3EC" />
                    </g>
                    <g filter={`url(#${p("f9")})`}>
                      <path d="M576.06 452.732C576.72 454.041 576.766 454.762 576.188 456C568.275 454.158 572.754 439.363 579.885 437.413C584.446 436.166 588.388 437.766 590 441.93L585.246 442.04C580.159 445.421 579.418 446.592 578.261 452.59C577.327 451.734 577.625 451.58 577.438 450.013L577.257 450.438L577.578 450.743L577.035 452.586L576.06 452.732Z" fill="#312E24" />
                    </g>
                    <g filter={`url(#${p("f10")})`}>
                      <path d="M576.49 452.759L575.948 452.886L575.475 452.235C575.11 444.84 575.121 438.674 584.935 442.224C580.259 445.556 579.577 446.709 578.514 452.619C577.655 451.776 577.929 451.624 577.757 450.08L577.591 450.499L577.886 450.8L577.387 452.615L576.49 452.759Z" fill="#534639" />
                    </g>
                  </>
                )}
              </g>
              </g>
              )}

              {/* Left cheek */}
              <path d="M354.002 488.785C366.292 488.07 381.734 490.477 385.001 505.019C386.026 509.579 385.143 514.363 382.556 518.257C378.409 524.432 372.217 526.795 365.337 528.245C353.923 529.158 338.873 527.064 334.774 514.24C333.375 509.718 333.887 504.821 336.192 500.686C339.888 493.968 346.962 490.735 354.002 488.785Z" fill="#F9A6A0" />
              <g filter={`url(#${p("f11")})`}>
                <path d="M368 494C373.244 494.048 380.363 498.673 380 504C375.832 504.091 367.526 498.087 368 494Z" fill="#FDC3BF" />
              </g>

              {/* Right cheek */}
              <path d="M626.146 494.285C641.877 485.407 671.147 495.187 664.86 516.522C657.951 539.968 605.954 533.98 615.075 505.471C615.73 503.36 618.571 499.408 620.251 497.867C621.588 496.68 624.466 495.224 626.146 494.285Z" fill="#EF928B" />
              <g filter={`url(#${p("f12")})`}>
                <path d="M632.013 497C626.77 497.048 619.65 501.673 620.013 507C624.181 507.091 632.487 501.087 632.013 497Z" fill="#FDC3BF" />
              </g>

              {/* Mouth — normal smile fades to a concerned "hmm" when thinking */}
              {!talking && (
                <>
                  {/* Normal closed smile — fades out as thinking kicks in */}
                  <g opacity={thinking ? Math.max(0, 1 - thinkProgress * 2.2) : 1}>
                    <path d="M471.504 494.784C471.5 491.499 475 489 478.416 490.134C480.5 491.5 480.95 493.63 482.461 495.842C489.371 505.97 498.06 507.141 509.126 502.936C514.767 498.973 514.929 497.593 518.612 491.664C528.419 484.735 532.464 504.579 511.184 513.085C503.114 516.238 494.124 516.055 486.187 512.586C478.627 509.187 473.047 503.065 471.504 494.784Z" fill="#1C170B" />
                    <path d="M509.127 502.936C514.767 498.973 514.929 497.593 518.612 491.664L520.234 492.572C521.198 496.986 512.309 506.706 507.958 505.884L507.711 505.234L509.127 502.936Z" fill="#312E24" />
                  </g>
                  {/* Thinking / "hmm" mouth — asymmetric slight frown, fades in */}
                  {thinking && (
                    <path
                      d="M480,509 Q490,518 503,511 Q512,505 519,509"
                      stroke="#1C170B"
                      strokeWidth="4.5"
                      strokeLinecap="round"
                      fill="none"
                      opacity={Math.min(1, thinkProgress * 2.5)}
                    />
                  )}
                </>
              )}

              {/* Talking mouth — pivot at top edge (y=508).
                  Whole group scales downward so mouth opens like a jaw drop.
                  Tongue is sized to stay within mouth walls at all mouthOpen values:
                    at cx=495 cy=532 rx=24, the widest point (y=532) sits inside the
                    ~73px-wide mouth cavity, with ≥8px margin on each side. */}
              {talking && (
                <g transform={`translate(495,508) scale(1,${mouthOpen}) translate(-495,-508)`}>
                  {/* Outer mouth: wide rounded top, deep U-curve bottom */}
                  <path
                    d="M453,508 Q453,501 463,501 L527,501 Q537,501 537,508 Q537,532 520,546 Q495,557 470,546 Q453,532 453,508 Z"
                    fill="#1C170B"
                  />
                  {/* Tongue — centered, safely inside mouth at full open.
                      Fades in so it's invisible while mouth is nearly closed. */}
                  <ellipse cx={495} cy={532} rx={24} ry={10} fill="#C03030" opacity={tongueOpacity} />
                  {/* Specular highlight on tongue */}
                  <ellipse cx={483} cy={526} rx={7} ry={4} fill="#E07070" opacity={tongueOpacity * 0.85} />
                </g>
              )}
            </g>
          )}

          {/* Recording face — pulsing dot, centered at (495, 495): 25px lower + 70% scale.
              Transform: place at target center → scale → undo RecordingFace's own offset (520,555). */}
          {face === "recording" && (
            <g transform="translate(495, 495) scale(0.7) translate(-520, -555)">
              <RecordingFace frame={frame} fps={fps} color={recordingColor} />
            </g>
          )}

          {/* Loading face — spinning ring, same center/scale as recording dot (495, 495, 70%). */}
          {face === "loading" && (
            <g transform="translate(495, 495) scale(0.7) translate(-520, -555)">
              <LoadingFace frame={frame} fps={fps} color={loadingColor} />
            </g>
          )}

          {/* Zzz — floating letters that drift up after mascot falls asleep */}
          {isAsleep && (
            <>
              <text x={z1.x} y={z1.y} fontSize={z1.fontSize} fontFamily="Arial Rounded MT Bold, Arial Black, Arial, sans-serif" fontWeight="900" fill="#5B9BD5" opacity={z1.opacity} textAnchor="middle">Z</text>
              <text x={z2.x} y={z2.y} fontSize={z2.fontSize} fontFamily="Arial Rounded MT Bold, Arial Black, Arial, sans-serif" fontWeight="900" fill="#4A8AC4" opacity={z2.opacity} textAnchor="middle">Z</text>
              <text x={z3.x} y={z3.y} fontSize={z3.fontSize} fontFamily="Arial Rounded MT Bold, Arial Black, Arial, sans-serif" fontWeight="900" fill="#3A7AB3" opacity={z3.opacity} textAnchor="middle">Z</text>
            </>
          )}
        </g>
      </svg>
    </AbsoluteFill>
  );
};
