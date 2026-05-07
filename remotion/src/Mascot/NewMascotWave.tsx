import React from "react";
import {
  AbsoluteFill,
  Easing,
  interpolate,
  useCurrentFrame,
  useVideoConfig,
} from "remotion";

/**
 * NewMascotWave — new-mascot.svg paths with a "say hi" wave animation.
 * No pop-in: mascot is idle on screen from frame 0.
 *   • body bob, head drift + squash
 *   • right arm rises over ~25 frames then waves enthusiastically in a loop
 *   • left arm gentle idle sway
 *   • legs rock at hips
 *   • eyes blink every ~2.6 s
 *   • closed smile
 *   • cheek warmth pulse
 *   • ground shadow
 */
export const NewMascotWave: React.FC<Props> = ({ accessory = "none" }) => {
  const frame = useCurrentFrame();
  const { fps, width, height } = useVideoConfig();
  const p = (k: string) => `nmw-${k}`;

  const easeInOut = Easing.inOut(Easing.cubic);

  // ── Body bob ─────────────────────────────────────────────────────────────────
  const bob = Math.sin((frame / fps) * Math.PI * 1.2) * 16;

  // ── Head drift + squash ───────────────────────────────────────────────────────
  const dotPhase = (frame / fps) * Math.PI * 1.0;
  const headDx = Math.sin(dotPhase * 0.7) * 7;
  const headDy = Math.sin(dotPhase) * 11;
  const press = Math.max(0, Math.sin(dotPhase));
  const headSquashY = 1 - 0.08 * press;
  const headSquashX = 1 + 0.05 * press;

  // ── Left arm — gentle idle sway (unchanged) ───────────────────────────────────
  const leftSway = Math.sin((frame / fps) * Math.PI * 1.6) * 7;

  // ── Right arm — rise then wave ────────────────────────────────────────────────
  // Phase 1 (0–25 f): arm smoothly rises from rest to raised "hi" position.
  const riseFrames = 25;
  const raiseProgress = interpolate(frame, [0, riseFrames], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
    easing: Easing.out(Easing.cubic),
  });
  const raisedAngle = -65; // degrees: brings arm up to ~"hi" position

  // Phase 2 (25 f+): enthusiastic wave oscillation around the raised position.
  const wavePeriod = Math.round(fps * 1.3);
  const waveFrame = frame >= riseFrames ? (frame - riseFrames) % wavePeriod : 0;
  const waveOscillate =
    frame >= riseFrames
      ? interpolate(
          waveFrame,
          [
            0,
            wavePeriod * 0.25,
            wavePeriod * 0.5,
            wavePeriod * 0.75,
            wavePeriod,
          ],
          [0, -22, -2, -24, 0],
          {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: easeInOut,
          },
        )
      : 0;

  // Combine: interpolate from idle sway → raised, then add wave on top.
  const rightArmAngle =
    interpolate(raiseProgress, [0, 1], [0, raisedAngle]) + waveOscillate;

  // ── Legs — subtle hip tilt ────────────────────────────────────────────────────
  const leftLegTilt = Math.sin((frame / fps) * Math.PI * 0.75) * 2.5;
  const rightLegTilt = -leftLegTilt;

  // ── Blink every ~2.6 s ────────────────────────────────────────────────────────
  const blinkPeriod = Math.round(fps * 2.6);
  const blinkOffset = Math.round(blinkPeriod / 2);
  const inBlink = (frame + blinkOffset) % blinkPeriod < 6;
  const eyeScale = inBlink ? 0.12 : 1;

  // ── Cheek warmth pulse ────────────────────────────────────────────────────────
  const cheekOpacity = 0.82 + Math.sin((frame / fps) * Math.PI * 1.1 + 1.0) * 0.1;

  const size = Math.min(width, height) * 0.82;

  return (
    <AbsoluteFill style={{ justifyContent: "center", alignItems: "center" }}>
      <svg
        width={size}
        height={size}
        viewBox="0 0 1200 1200"
        style={{ overflow: "visible" }}
      >
        <defs>
          {/* Ground shadow */}
          <radialGradient id={p("ground")} cx="0.5" cy="0.5" r="0.5">
            <stop offset="0%" stopColor="#000000" stopOpacity="0.3" />
            <stop offset="100%" stopColor="#000000" stopOpacity="0" />
          </radialGradient>

          {/* f0: left leg */}
          <filter id={p("f0")} x="270" y="903.746" width="252.3" height="208.252" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="18.7402" dy="-29"/><feGaussianBlur stdDeviation="10.05"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.962384 0 0 0 0 0.860378 0 0 0 0 0.484572 0 0 0 1 0"/>
            <feBlend mode="normal" in2="shape" result="e1"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="-22" dy="-30"/><feGaussianBlur stdDeviation="15"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.797063 0 0 0 0 0.575703 0 0 0 0 0.0980312 0 0 0 1 0"/>
            <feBlend mode="normal" in2="e1" result="e2"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="20" dy="32"/><feGaussianBlur stdDeviation="15"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.797063 0 0 0 0 0.575703 0 0 0 0 0.0980312 0 0 0 0.7 0"/>
            <feBlend mode="normal" in2="e2" result="e3"/>
            <feTurbulence type="fractalNoise" baseFrequency="0.999" numOctaves={3} seed={8703}/>
            <feDisplacementMap in="e3" scale={8.82} xChannelSelector="R" yChannelSelector="G" width="100%" height="100%"/>
          </filter>

          {/* f1: right leg */}
          <filter id={p("f1")} x="663.208" y="924.513" width="254.584" height="203.043" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="-28" dy="-29"/><feGaussianBlur stdDeviation="10.05"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.962384 0 0 0 0 0.860378 0 0 0 0 0.484572 0 0 0 1 0"/>
            <feBlend mode="normal" in2="shape" result="e1"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="22" dy="-18"/><feGaussianBlur stdDeviation="15"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.797063 0 0 0 0 0.575703 0 0 0 0 0.0980312 0 0 0 1 0"/>
            <feBlend mode="normal" in2="e1" result="e2"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="-37" dy="31"/><feGaussianBlur stdDeviation="15"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.797063 0 0 0 0 0.575703 0 0 0 0 0.0980312 0 0 0 0.7 0"/>
            <feBlend mode="normal" in2="e2" result="e3"/>
            <feTurbulence type="fractalNoise" baseFrequency="0.999" numOctaves={3} seed={8703}/>
            <feDisplacementMap in="e3" scale={8.82} xChannelSelector="R" yChannelSelector="G" width="100%" height="100%"/>
          </filter>

          {/* f2: body */}
          <filter id={p("f2")} x="212" y="276.635" width="765.268" height="762.13" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="17" dy="28"/><feGaussianBlur stdDeviation="10.45"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.962384 0 0 0 0 0.860378 0 0 0 0 0.484572 0 0 0 1 0"/>
            <feBlend mode="normal" in2="shape" result="e1"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="-27" dy="-22"/><feGaussianBlur stdDeviation="29.75"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.797063 0 0 0 0 0.575703 0 0 0 0 0.0980312 0 0 0 1 0"/>
            <feBlend mode="normal" in2="e1" result="e2"/>
            <feTurbulence type="fractalNoise" baseFrequency="0.999" numOctaves={3} seed={8703}/>
            <feDisplacementMap in="e2" scale={8} xChannelSelector="R" yChannelSelector="G" width="100%" height="100%"/>
          </filter>

          {/* f3: head circle */}
          <filter id={p("f3")} x="516.159" y="91.7727" width="201.227" height="204.682" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="7.77273" dy="1.72727"/><feGaussianBlur stdDeviation="4.87955"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 1 0 0 0 0 1 0 0 0 0 1 0 0 0 1 0"/>
            <feBlend mode="normal" in2="e1" result="e2"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="-1.72727" dy="-11.2273"/><feGaussianBlur stdDeviation="17.0136"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.797063 0 0 0 0 0.575703 0 0 0 0 0.0980312 0 0 0 1 0"/>
            <feBlend mode="normal" in2="e1" result="e2"/>
            <feTurbulence type="fractalNoise" baseFrequency="0.999" numOctaves={3} seed={8703}/>
            <feDisplacementMap in="e2" scale={6.91} xChannelSelector="R" yChannelSelector="G" width="100%" height="100%"/>
          </filter>

          {/* f4–f5: neck shadows */}
          <filter id={p("f4")} x="545.114" y="277.5" width="153.771" height="66.8594" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="11.25"/>
          </filter>
          <filter id={p("f5")} x="556.59" y="255.945" width="123.537" height="57.3711" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="11.25"/>
          </filter>

          {/* f6: left arm */}
          <filter id={p("f6")} x="260.072" y="593.812" width="155.094" height="272.387" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="1" dy="-20"/><feGaussianBlur stdDeviation="7.55"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.973501 0 0 0 0 0.909066 0 0 0 0 0.671677 0 0 0 1 0"/>
            <feBlend mode="normal" in2="shape" result="e1"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="3" dy="-8"/><feGaussianBlur stdDeviation="3.55"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.796078 0 0 0 0 0.576471 0 0 0 0 0.0980392 0 0 0 0.8 0"/>
            <feBlend mode="normal" in2="e1" result="e2"/>
            <feTurbulence type="fractalNoise" baseFrequency="0.999" numOctaves={3} seed={8703}/>
            <feDisplacementMap in="e2" scale={8} xChannelSelector="R" yChannelSelector="G" width="100%" height="100%"/>
          </filter>

          {/* f7: right arm */}
          <filter id={p("f7")} x="766.614" y="593.9" width="155.094" height="272.387" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="1" dy="-20"/><feGaussianBlur stdDeviation="7.55"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.973501 0 0 0 0 0.909066 0 0 0 0 0.671677 0 0 0 1 0"/>
            <feBlend mode="normal" in2="shape" result="e1"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dy="-12"/><feGaussianBlur stdDeviation="3.55"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.796078 0 0 0 0 0.576471 0 0 0 0 0.0980392 0 0 0 0.8 0"/>
            <feBlend mode="normal" in2="e1" result="e2"/>
            <feTurbulence type="fractalNoise" baseFrequency="0.999" numOctaves={3} seed={8703}/>
            <feDisplacementMap in="e2" scale={8} xChannelSelector="R" yChannelSelector="G" width="100%" height="100%"/>
          </filter>

          {/* f8–f9: left eye highlights */}
          <filter id={p("f8")} x="511.832" y="471.891" width="25.0343" height="28.893" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.65"/>
          </filter>
          <filter id={p("f9")} x="511.914" y="472.3" width="22.4" height="23.4" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.35"/>
          </filter>

          {/* f10–f12: right eye highlights */}
          <filter id={p("f10")} x="692.473" y="473.359" width="27.0395" height="29.1125" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.95"/>
          </filter>
          <filter id={p("f11")} x="692.914" y="474.301" width="19.4" height="20.4" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.35"/>
          </filter>
          <filter id={p("f12")} x="696.282" y="478.493" width="10.9676" height="13.0934" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.35"/>
          </filter>

          {/* f13–f14: cheek highlights */}
          <filter id={p("f13")} x="487.795" y="530.199" width="15.6325" height="13.601" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.9"/>
          </filter>
          <filter id={p("f14")} x="739.814" y="533.2" width="15.6325" height="13.602" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.9"/>
          </filter>
        </defs>

        {/* Ground shadow */}
        <g transform={`translate(600, 1110) scale(${1 - bob / 700}, 1)`}>
          <ellipse cx={0} cy={0} rx={340} ry={30} fill={`url(#${p("ground")})`} />
        </g>

        {/* Everything bobs together */}
        <g transform={`translate(0, ${bob})`}>

          {/* Left leg */}
          <g transform={`rotate(${leftLegTilt}, 405, 950)`}>
            <path
              d="M292.201 1015.78C293.455 994.457 312.319 966.113 316.5 963.499C334.06 913.327 444.288 939.476 502.3 949.928C501.777 961.949 500.419 988.499 499.164 998.533C497.91 1008.57 492.37 1026.76 489.757 1034.59C486.098 1042.43 475.019 1060.31 459.967 1069.09C441.152 1080.06 353.349 1086.34 331.399 1078.5C315.333 1072.76 304.167 1060.46 299.581 1053.6C298.808 1052.45 297.999 1051.32 297.314 1050.11C294.649 1045.41 291.115 1034.24 292.201 1015.78Z"
              fill="#F7D145"
              filter={`url(#${p("f0")})`}
            />
          </g>

          {/* Right leg */}
          <g transform={`rotate(${rightLegTilt}, 793, 944)`}>
            <path
              d="M895.486 1040.5C897.341 1019.22 890.29 1007.52 886.532 1004.32C876.441 952.132 756.088 942.925 697.173 944.845C695.945 956.814 693.435 983.28 693.219 993.39C693.004 1003.5 695.845 1022.3 697.292 1030.44C699.774 1038.72 708.141 1058.02 721.759 1068.89C738.781 1082.48 824.743 1101.43 847.599 1096.86C865.884 1093.21 879.516 1080.94 884.046 1075.27C887.087 1072.55 893.632 1061.78 895.486 1040.5Z"
              fill="#F7D145"
              filter={`url(#${p("f1")})`}
            />
          </g>

          {/* Body */}
          <path
            d="M392.162 420.715C297.483 517.648 207.754 692.574 249.529 867.518C266.887 919.372 286.816 949.977 344.549 979.976C374.951 995.773 449.114 988.501 497.158 959.665L567.009 928.457C612.356 911.852 631.186 914.413 660.114 927.193C698.644 948.414 709.114 969.501 770.821 1002.22C851.101 1044.79 914.741 994.042 939.128 927.193C996.422 780.916 936.128 460.979 771.945 348.48C637.668 264.595 524.618 285.227 392.162 420.715Z"
            fill="#F7D145"
            filter={`url(#${p("f2")})`}
          />

          {/* Left arm — gentle idle sway */}
          <g transform={`rotate(${leftSway}, 408, 747)`}>
            <path
              d="M379.314 811.068C393.343 774.698 394.601 766.506 408.844 747.133C421.253 730.255 381.457 665.746 347.846 615.586C341.476 606.08 327.007 606.903 321.52 616.945C298.126 659.76 258.658 732.31 264.692 780.936C277.832 886.842 350.044 880.851 379.314 811.068Z"
              fill="#F7D145"
              filter={`url(#${p("f6")})`}
            />
          </g>

          {/* Right arm — rises then waves */}
          <g transform={`rotate(${rightArmAngle}, 773, 747)`}>
            <path
              d="M802.466 811.156C788.437 774.786 787.179 766.594 772.935 747.221C760.527 730.343 800.323 665.834 833.934 615.674C840.304 606.167 854.773 606.991 860.26 617.033C883.654 659.848 923.122 732.398 917.088 781.024C903.947 886.93 831.736 880.939 802.466 811.156Z"
              fill="#F7D145"
              filter={`url(#${p("f7")})`}
            />
          </g>

          {/* Neck shadows */}
          <g opacity={0.4} filter={`url(#${p("f4")})`}>
            <path d="M571.99 308.172C585.656 302.005 623.69 293.372 666.49 308.172C719.99 326.672 537.49 326.172 571.99 308.172Z" fill="#B23C05"/>
          </g>
          <g opacity={0.4} filter={`url(#${p("f5")})`}>
            <path d="M655.114 283.498C646.57 286.601 611.557 295.334 584.8 287.887C551.354 278.577 676.682 274.441 655.114 283.498Z" fill="#B23C05"/>
          </g>

          {/* Head — drifts + squashes around its center (614.614, 198) */}
          <g transform={
            `translate(${headDx}, ${headDy}) ` +
            `translate(614.614 198) scale(${headSquashX} ${headSquashY}) translate(-614.614 -198)`
          }>
            <circle
              cx={614.614}
              cy={198}
              r={95}
              fill="#F7D145"
              filter={`url(#${p("f3")})`}
            />
          </g>

          {/* Left eye */}
          <g transform={`translate(535, 503) scale(1, ${eyeScale}) translate(-535, -503)`}>
            <path
              d="M533.094 466C541.293 466 544.614 470 546.022 472.321C553.07 480.807 556.062 488.812 556.901 499.939C558.145 516.451 550.195 539.025 530.79 539.922C524.521 540.212 518.397 537.978 513.791 533.714C494.582 516.168 501.07 466.811 533.094 466Z"
              fill="#1C170B"
            />
            {!inBlink && (
              <>
                <g filter={`url(#${p("f8")})`}>
                  <path d="M524.204 473.31C526.728 473.115 527.733 473.015 529.84 474.218C531.063 475.699 530.909 476.305 530.981 478.116C531.794 478.625 532.513 479.111 533.309 479.647L533.519 480.956C540.629 494.194 527.648 506.295 518.619 495.028C508.723 495.791 514.641 483.603 517.66 479.344C519.653 476.531 521.483 475.302 524.204 473.31Z" fill="#FAF3EC"/>
                </g>
                <g filter={`url(#${p("f9")})`}>
                  <path d="M524.019 473.12C526.619 472.923 527.655 472.822 529.825 474.033C531.085 475.522 530.926 476.132 531 477.954C531.838 478.465 532.578 478.954 533.398 479.493L533.614 480.811C530.172 479.118 528.239 477.187 524.155 478.654C517.387 481.086 515.882 489.112 518.266 494.966C508.073 495.733 514.169 483.473 517.278 479.189C519.331 476.36 521.216 475.123 524.019 473.12Z" fill="#3A372F"/>
                </g>
              </>
            )}
          </g>

          {/* Right eye */}
          <g transform={`translate(715, 505) scale(1, ${eyeScale}) translate(-715, -505)`}>
            <path
              d="M710.984 466.707C743.481 466.524 752.608 531.599 715.966 540.664C677.3 542.42 676.07 471.12 710.984 466.707Z"
              fill="#1C170B"
            />
            {!inBlink && (
              <>
                <g filter={`url(#${p("f10")})`}>
                  <path d="M698.105 490.76C698.712 492.05 698.755 492.76 698.223 493.98C690.948 492.165 695.066 477.587 701.622 475.665C705.815 474.437 709.439 476.014 710.92 480.116C714.233 482.138 716.461 484.011 717.363 488.05C717.97 490.792 717.46 493.662 715.946 496.028C710.652 504.355 701.917 500.461 700.129 490.62C699.27 489.776 699.544 489.625 699.372 488.08L699.206 488.5L699.501 488.801L699.002 490.616L698.105 490.76Z" fill="#FAF3EC"/>
                </g>
                <g filter={`url(#${p("f11")})`}>
                  <path d="M697.674 490.733C698.334 492.042 698.381 492.763 697.802 494.001C689.889 492.159 694.368 477.364 701.499 475.414C706.06 474.167 710.002 475.767 711.614 479.931L706.86 480.041C701.774 483.422 701.032 484.593 699.876 490.591C698.941 489.735 699.239 489.581 699.052 488.014L698.871 488.439L699.192 488.744L698.649 490.587L697.674 490.733Z" fill="#312E24"/>
                </g>
                <g filter={`url(#${p("f12")})`}>
                  <path d="M698.104 490.76L697.562 490.887L697.09 490.236C696.725 482.841 696.735 476.675 706.55 480.225C701.873 483.557 701.191 484.71 700.128 490.62C699.269 489.777 699.543 489.625 699.371 488.081L699.205 488.5L699.5 488.801L699.001 490.616L698.104 490.76Z" fill="#534639"/>
                </g>
              </>
            )}
          </g>

          {/* Left cheek */}
          <g opacity={cheekOpacity}>
            <path
              d="M475.616 526.784C487.907 526.069 503.348 528.476 506.615 543.018C507.64 547.578 506.757 552.362 504.17 556.256C500.024 562.431 493.832 564.794 486.951 566.244C475.537 567.157 460.487 565.063 456.388 552.239C454.989 547.717 455.502 542.82 457.806 538.685C461.502 531.967 468.576 528.734 475.616 526.784Z"
              fill="#F9A6A0"
            />
            <g filter={`url(#${p("f13")})`}>
              <path d="M489.615 531.999C494.858 532.047 501.977 536.672 501.614 541.999C497.446 542.09 489.141 536.086 489.615 531.999Z" fill="#FDC3BF"/>
            </g>
          </g>

          {/* Right cheek */}
          <g opacity={cheekOpacity}>
            <path
              d="M747.76 532.285C763.491 523.407 792.762 533.187 786.475 554.522C779.565 577.968 727.568 571.98 736.69 543.471C737.345 541.36 740.185 537.408 741.865 535.866C743.202 534.68 746.08 533.224 747.76 532.285Z"
              fill="#EF928B"
            />
            <g filter={`url(#${p("f14")})`}>
              <path d="M753.627 535C748.384 535.048 741.265 539.673 741.627 545C745.795 545.091 754.101 539.087 753.627 535Z" fill="#FDC3BF"/>
            </g>
          </g>

          {/* Closed smile */}
          <path
            d="M593.119 536.783C593.114 533.498 596.614 530.999 600.03 532.133C602.114 533.499 602.564 535.629 604.075 537.841C610.985 547.969 619.674 549.14 630.741 544.935C636.381 540.972 636.543 539.592 640.226 533.663C650.033 526.734 654.078 546.578 632.798 555.084C624.729 558.237 615.738 558.054 607.801 554.585C600.241 551.186 594.662 545.064 593.119 536.783Z"
            fill="#1C170B"
          />
          <path
            d="M630.741 544.935C636.381 540.972 636.544 539.592 640.227 533.663L641.848 534.571C642.812 538.985 633.924 548.705 629.573 547.883L629.325 547.233L630.741 544.935Z"
            fill="#312E24"
          />

        </g>
      </svg>
      {accessory !== "none" && (
        <AbsoluteFill style={{ justifyContent: "center", alignItems: "center", pointerEvents: "none" }}>
          <Img
            src={staticFile(`${accessory}.svg`)}
            style={{ width: size, height: size, objectFit: "contain" }}
          />
        </AbsoluteFill>
      )}
    </AbsoluteFill>
  );
};
