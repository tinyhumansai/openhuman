import React from "react";
import { AbsoluteFill, useCurrentFrame, useVideoConfig } from "remotion";
import { RecordingFace } from "../Ghosty/lib/RecordingFace";

/**
 * Black recording mascot — uses exact paths and filters from BlackIdelmascot.svg
 * with the same bob, head-drift, arm-sway animations as the black idle,
 * but replaces the face with a pulsing red recording dot.
 */
export const BlackMascotRecording: React.FC = () => {
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

  // Left arm gentle sway.
  const leftSway = Math.sin((frame / fps) * Math.PI * 1.6) * 7;
  // Steady right arm sway — mirrors left arm with slight phase offset.
  const steadySway = Math.sin((frame / fps) * Math.PI * 1.6 + 0.3) * 6;

  const size = Math.min(width, height) * 0.85;

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
          <radialGradient id="bmr-ground" cx="0.5" cy="0.5" r="0.5">
            <stop offset="0%" stopColor="#000000" stopOpacity="0.35" />
            <stop offset="100%" stopColor="#000000" stopOpacity="0" />
          </radialGradient>

          {/* Body filter — from BlackIdelmascot.svg filter0_iig */}
          <filter id="bmr-f0" x="90.3867" y="238.634" width="765.266" height="762.131" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha" />
            <feOffset dx="17" dy="28" />
            <feGaussianBlur stdDeviation="10.45" />
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1" />
            <feColorMatrix type="matrix" values="0 0 0 0 0.439078 0 0 0 0 0.439078 0 0 0 0 0.439078 0 0 0 1 0" />
            <feBlend mode="normal" in2="shape" result="effect1_innerShadow" />
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha" />
            <feOffset dx="-27" dy="-22" />
            <feGaussianBlur stdDeviation="29.75" />
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1" />
            <feColorMatrix type="matrix" values="0 0 0 0 0.0229492 0 0 0 0 0.0207891 0 0 0 0 0.0161271 0 0 0 1 0" />
            <feBlend mode="normal" in2="effect1_innerShadow" result="effect2_innerShadow" />
            <feTurbulence type="fractalNoise" baseFrequency="0.999" numOctaves={3} seed={8703} />
            <feDisplacementMap in="effect2_innerShadow" scale={8} xChannelSelector="R" yChannelSelector="G" result="displacedImage" width="100%" height="100%" />
            <feMerge><feMergeNode in="displacedImage" /></feMerge>
          </filter>

          {/* Head filter — from BlackIdelmascot.svg filter1_iig */}
          <filter id="bmr-f1" x="379.002" y="22" width="233" height="237" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha" />
            <feOffset dx="9" dy="2" />
            <feGaussianBlur stdDeviation="5.65" />
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1" />
            <feColorMatrix type="matrix" values="0 0 0 0 0.439216 0 0 0 0 0.439216 0 0 0 0 0.439216 0 0 0 1 0" />
            <feBlend mode="normal" in2="shape" result="effect1_innerShadow" />
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha" />
            <feOffset dx="-2" dy="-13" />
            <feGaussianBlur stdDeviation="19.7" />
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1" />
            <feColorMatrix type="matrix" values="0 0 0 0 0.0235294 0 0 0 0 0.0196078 0 0 0 0 0.0156863 0 0 0 1 0" />
            <feBlend mode="normal" in2="effect1_innerShadow" result="effect2_innerShadow" />
            <feTurbulence type="fractalNoise" baseFrequency="0.999" numOctaves={3} seed={8703} />
            <feDisplacementMap in="effect2_innerShadow" scale={8} xChannelSelector="R" yChannelSelector="G" result="displacedImage" width="100%" height="100%" />
            <feMerge><feMergeNode in="displacedImage" /></feMerge>
          </filter>

          {/* Neck shadow filters */}
          <filter id="bmr-f2" x="423.502" y="239.5" width="153.77" height="66.86" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feGaussianBlur stdDeviation="11.25" />
          </filter>
          <filter id="bmr-f3" x="434.979" y="217.947" width="123.535" height="57.3708" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feGaussianBlur stdDeviation="11.25" />
          </filter>

          {/* Left arm filter — from BlackIdelmascot.svg filter4_iig */}
          <filter id="bmr-f4" x="138.459" y="555.812" width="155.094" height="272.386" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha" />
            <feOffset dx="1" dy="-20" />
            <feGaussianBlur stdDeviation="7.55" />
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1" />
            <feColorMatrix type="matrix" values="0 0 0 0 0.439216 0 0 0 0 0.439216 0 0 0 0 0.439216 0 0 0 1 0" />
            <feBlend mode="normal" in2="shape" result="effect1_innerShadow" />
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha" />
            <feOffset dx="3" dy="-8" />
            <feGaussianBlur stdDeviation="3.55" />
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1" />
            <feColorMatrix type="matrix" values="0 0 0 0 0.0235294 0 0 0 0 0.0196078 0 0 0 0 0.0156863 0 0 0 0.8 0" />
            <feBlend mode="normal" in2="effect1_innerShadow" result="effect2_innerShadow" />
            <feTurbulence type="fractalNoise" baseFrequency="0.999" numOctaves={3} seed={8703} />
            <feDisplacementMap in="effect2_innerShadow" scale={8} xChannelSelector="R" yChannelSelector="G" result="displacedImage" width="100%" height="100%" />
            <feMerge><feMergeNode in="displacedImage" /></feMerge>
          </filter>

          {/* Right steady arm filter — from BlackIdelmascot.svg filter5_iig */}
          <filter id="bmr-f5" x="645" y="555.9" width="155.094" height="272.386" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha" />
            <feOffset dx="1" dy="-20" />
            <feGaussianBlur stdDeviation="7.55" />
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1" />
            <feColorMatrix type="matrix" values="0 0 0 0 0.439216 0 0 0 0 0.439216 0 0 0 0 0.439216 0 0 0 1 0" />
            <feBlend mode="normal" in2="shape" result="effect1_innerShadow" />
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha" />
            <feOffset dy="-8" />
            <feGaussianBlur stdDeviation="3.55" />
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1" />
            <feColorMatrix type="matrix" values="0 0 0 0 0.0235294 0 0 0 0 0.0196078 0 0 0 0 0.0156863 0 0 0 0.8 0" />
            <feBlend mode="normal" in2="effect1_innerShadow" result="effect2_innerShadow" />
            <feTurbulence type="fractalNoise" baseFrequency="0.999" numOctaves={3} seed={8703} />
            <feDisplacementMap in="effect2_innerShadow" scale={8} xChannelSelector="R" yChannelSelector="G" result="displacedImage" width="100%" height="100%" />
            <feMerge><feMergeNode in="displacedImage" /></feMerge>
          </filter>
        </defs>

        {/* Ground shadow */}
        <g transform={`translate(500, 975) scale(${1 - bob / 600}, 1)`}>
          <ellipse cx={0} cy={0} rx={300} ry={28} fill="url(#bmr-ground)" />
        </g>

        {/* Everything bobs together */}
        <g transform={`translate(0, ${bob})`}>

          {/* Head — drifts + squashes independently */}
          <g transform={
            `translate(${dotDx}, ${dotDy}) ` +
            `translate(493 145) scale(${dotSquashX} ${dotSquashY}) translate(-493 -145)`
          }>
            <g filter="url(#bmr-f1)">
              <circle cx="493.002" cy="145" r="110" fill="#3A3A3A" />
            </g>
          </g>

          {/* Body */}
          <g filter="url(#bmr-f0)">
            <path d="M270.549 382.715C175.87 479.648 86.1412 654.573 127.916 829.517C145.273 881.371 165.203 911.977 222.936 941.975C253.338 957.772 327.501 950.5 375.545 921.664L445.395 890.457C490.743 873.851 509.573 876.412 538.501 889.192C577.03 910.414 587.501 931.5 649.208 964.222C729.488 1006.79 793.127 956.041 817.515 889.192C874.809 742.915 814.515 422.979 650.332 310.48C516.055 226.594 403.004 247.226 270.549 382.715Z" fill="#3A3A3A" />
          </g>

          {/* Right steady arm — gentle sway */}
          <g transform={`rotate(${steadySway}, 655, 709)`}>
            <g filter="url(#bmr-f5)">
              <path d="M680.852 773.156C666.823 736.786 665.565 728.594 651.322 709.221C638.913 692.343 678.709 627.834 712.32 577.674C718.69 568.167 733.159 568.991 738.646 579.033C762.04 621.848 801.508 694.398 795.474 743.024C782.334 848.93 710.122 842.939 680.852 773.156Z" fill="#3A3A3A" />
            </g>
          </g>

          {/* Left arm — gentle sway */}
          <g transform={`rotate(${leftSway}, 290, 700)`}>
            <g filter="url(#bmr-f4)">
              <path d="M257.701 773.068C271.729 736.698 272.988 728.506 287.231 709.133C299.639 692.255 259.843 627.746 226.233 577.586C219.863 568.08 205.394 568.903 199.907 578.945C176.512 621.76 137.045 694.31 143.078 742.936C156.219 848.842 228.43 842.851 257.701 773.068Z" fill="#3A3A3A" />
            </g>
          </g>

          {/* Neck shadows */}
          <g opacity={0.4} filter="url(#bmr-f2)">
            <path d="M450.377 270.172C464.044 264.005 502.077 255.372 544.877 270.172C598.377 288.672 415.877 288.172 450.377 270.172Z" fill="#030100" />
          </g>
          <g opacity={0.4} filter="url(#bmr-f3)">
            <path d="M533.501 245.499C524.957 248.602 489.945 257.335 463.187 249.888C429.741 240.578 555.07 236.442 533.501 245.499Z" fill="white" />
          </g>

          {/* Recording face — pulsing dot, centered at (495, 495): 25px lower + 70% scale. */}
          <g transform="translate(495, 495) scale(0.7) translate(-520, -555)">
            <RecordingFace frame={frame} fps={fps} color="#ff3b30" />
          </g>
        </g>
      </svg>
    </AbsoluteFill>
  );
};
