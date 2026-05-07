import React from "react";
import {
  AbsoluteFill,
  Easing,
  interpolate,
  useCurrentFrame,
  useVideoConfig,
} from "remotion";

/**
 * NewMascotLove — idelMascot.svg style idle animation with a heart-eye moment.
 *
 * Timeline (270 frames / 9 s @ 30 fps):
 *   0  – 89  : normal idle  (body bob, head drift, gentle arm sway, blink)
 *   90 – 120 : heart eyes fade IN  (cross-fade over 30 frames)
 *   120 – 210: heart eyes pulse rhythmically, cheeks flush, mini hearts float up
 *   210 – 240: heart eyes fade OUT (cross-fade back to round eyes)
 *   240 – 270: normal idle again  → clean loop
 *
 * Heart eye paths are from the love-face SVG (730×953 viewBox, head cx=379.614 cy=114).
 * They are placed into the 1000×1000 idelMascot coordinate space via
 * translate(+113.386, +31)  — the exact difference between the two head-circle centres.
 */
export const NewMascotLove: React.FC<Props> = ({ accessory = "none" }) => {
  const frame = useCurrentFrame();
  const { fps, width, height } = useVideoConfig();
  const p = (k: string) => `nmlv-${k}`;

  // ── Heart transition ────────────────────────────────────────────────────────
  const heartProgress = interpolate(
    frame,
    [90, 120, 210, 240],
    [0, 1, 1, 0],
    {
      extrapolateLeft: "clamp",
      extrapolateRight: "clamp",
      easing: Easing.inOut(Easing.cubic),
    }
  );
  const normalEyeOpacity = 1 - heartProgress;
  const heartEyeOpacity = heartProgress;

  // Heart pulse: 2 beats/s, amplitude grows with heartProgress
  const heartBeat = 1 + Math.sin((frame / fps) * Math.PI * 4) * 0.09 * heartProgress;

  // ── Body bob — classic idle rhythm ────────────────────────────────────────
  const bob = Math.sin((frame / fps) * Math.PI * 1.2) * 14;

  // ── Head drift + squash ──────────────────────────────────────────────────
  const dotPhase = (frame / fps) * Math.PI;
  const headDx = Math.sin(dotPhase * 0.7) * 6;
  const headDy = Math.sin(dotPhase) * 9;
  const press = Math.max(0, Math.sin(dotPhase));
  const headSquashY = 1 - 0.07 * press;
  const headSquashX = 1 + 0.04 * press;

  // ── Arms — gentle idle sway (offset phases for natural look) ─────────────
  const leftSway = Math.sin((frame / fps) * Math.PI * 1.3) * 7;
  const rightSway = Math.sin((frame / fps) * Math.PI * 1.3 + 1.0) * 6;

  // ── Blink — only during normal eye phase ────────────────────────────────
  const blinkPeriod = Math.round(fps * 2.8);
  const blinkOffset = Math.round(blinkPeriod / 2);
  const inBlink = heartProgress < 0.1 && (frame + blinkOffset) % blinkPeriod < 5;
  const eyeScaleNormal = inBlink ? 0.1 : 1;

  // ── Cheek — flushes more during heart phase ──────────────────────────────
  const cheekOpacity =
    0.82 + heartProgress * 0.15 +
    Math.sin((frame / fps) * Math.PI * 1.1 + 1.0) * 0.06;

  // ── Floating mini hearts ─────────────────────────────────────────────────
  // Three hearts staggered across the 90-frame heart window.
  // Returns {x, y (with bob), opacity (gated by heartProgress), scale}.
  const floatHeart = (startF: number, x: number, baseY: number, sz: number) => {
    const prog = interpolate(frame, [startF, startF + 48], [0, 1], {
      extrapolateLeft: "clamp",
      extrapolateRight: "clamp",
    });
    const y = baseY - 72 * prog + bob;
    const op =
      interpolate(
        frame,
        [startF, startF + 6, startF + 38, startF + 48],
        [0, 0.9, 0.9, 0],
        { extrapolateLeft: "clamp", extrapolateRight: "clamp" }
      ) * heartProgress;
    const sc =
      sz *
      interpolate(frame, [startF, startF + 8], [0.5, 1], {
        extrapolateLeft: "clamp",
        extrapolateRight: "clamp",
      });
    return { x, y, op, sc };
  };

  const hA = floatHeart(124, 415, 388, 0.9);
  const hB = floatHeart(152, 568, 382, 0.8);
  const hC = floatHeart(176, 490, 358, 1.0);

  const size = Math.min(width, height) * 0.82;

  // Heart-eye SVG → idelMascot coordinate shift
  const HX = 113.386;
  const HY = 31;

  return (
    <AbsoluteFill style={{ justifyContent: "center", alignItems: "center" }}>
      <svg
        width={size}
        height={size}
        viewBox="0 0 1000 1000"
        style={{ overflow: "visible" }}
      >
        <defs>
          {/* Ground shadow */}
          <radialGradient id={p("ground")} cx="0.5" cy="0.5" r="0.5">
            <stop offset="0%" stopColor="#000000" stopOpacity="0.28" />
            <stop offset="100%" stopColor="#000000" stopOpacity="0" />
          </radialGradient>

          {/* Body */}
          <filter id={p("f0")} x="90.3857" y="238.634" width="765.268" height="762.131" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
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

          {/* Head circle */}
          <filter id={p("f1")} x="379" y="22" width="233" height="237" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="9" dy="2"/><feGaussianBlur stdDeviation="5.65"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 1 0 0 0 0 1 0 0 0 0 1 0 0 0 1 0"/>
            <feBlend mode="normal" in2="shape" result="e1"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="-2" dy="-13"/><feGaussianBlur stdDeviation="19.7"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.797063 0 0 0 0 0.575703 0 0 0 0 0.0980312 0 0 0 1 0"/>
            <feBlend mode="normal" in2="e1" result="e2"/>
            <feTurbulence type="fractalNoise" baseFrequency="0.999" numOctaves={3} seed={8703}/>
            <feDisplacementMap in="e2" scale={8} xChannelSelector="R" yChannelSelector="G" width="100%" height="100%"/>
          </filter>

          {/* Neck shadows */}
          <filter id={p("f2")} x="423.5" y="239.5" width="153.771" height="66.8604" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="11.25"/>
          </filter>
          <filter id={p("f3")} x="434.976" y="217.946" width="123.537" height="57.3711" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="11.25"/>
          </filter>

          {/* Left arm */}
          <filter id={p("f4")} x="138.458" y="555.812" width="155.094" height="272.386" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
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

          {/* Right arm */}
          <filter id={p("f5")} x="645" y="555.9" width="155.094" height="272.386" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="1" dy="-20"/><feGaussianBlur stdDeviation="7.55"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.973501 0 0 0 0 0.909066 0 0 0 0 0.671677 0 0 0 1 0"/>
            <feBlend mode="normal" in2="shape" result="e1"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dy="-8"/><feGaussianBlur stdDeviation="3.55"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.796078 0 0 0 0 0.576471 0 0 0 0 0.0980392 0 0 0 0.8 0"/>
            <feBlend mode="normal" in2="e1" result="e2"/>
            <feTurbulence type="fractalNoise" baseFrequency="0.999" numOctaves={3} seed={8703}/>
            <feDisplacementMap in="e2" scale={8} xChannelSelector="R" yChannelSelector="G" width="100%" height="100%"/>
          </filter>

          {/* Normal left eye highlights */}
          <filter id={p("f6")} x="390.218" y="433.891" width="25.0343" height="28.893" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.65"/>
          </filter>
          <filter id={p("f7")} x="390.3" y="434.3" width="22.4" height="23.4" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.35"/>
          </filter>

          {/* Normal right eye highlights */}
          <filter id={p("f8")} x="570.859" y="435.358" width="27.0395" height="29.1125" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.95"/>
          </filter>
          <filter id={p("f9")} x="571.3" y="436.3" width="19.4" height="20.4" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.35"/>
          </filter>
          <filter id={p("f10")} x="574.668" y="440.492" width="10.9676" height="13.0943" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.35"/>
          </filter>

          {/* Cheek highlights */}
          <filter id={p("f11")} x="366.181" y="492.2" width="15.6325" height="13.601" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.9"/>
          </filter>
          <filter id={p("f12")} x="618.2" y="495.2" width="15.6325" height="13.601" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.9"/>
          </filter>

          {/* Pink glow blur (behind heart eyes) */}
          <filter id={p("glow")} x="-50%" y="-50%" width="200%" height="200%">
            <feGaussianBlur stdDeviation="12"/>
          </filter>
        </defs>

        {/* Ground shadow */}
        <ellipse
          cx={500} cy={978} rx={290} ry={22}
          fill={`url(#${p("ground")})`}
          transform={`scale(${1 - bob / 500}, 1)`}
          style={{ transformOrigin: "500px 978px" }}
        />

        {/* ── Floating mini hearts (bob-synced, gated by heartProgress) ─────── */}
        {[hA, hB, hC].map((h, i) => (
          <g key={i} transform={`translate(${h.x}, ${h.y}) scale(${h.sc})`} opacity={h.op}>
            {/* Simple heart path centred at (0,0) */}
            <path
              d="M0,-10 C-2,-18 -17,-17 -17,-6 C-17,3 0,13 0,13 C0,13 17,3 17,-6 C17,-17 2,-18 0,-10 Z"
              fill="#E8405A"
            />
          </g>
        ))}

        {/* ── Everything bobs together ──────────────────────────────────────── */}
        <g transform={`translate(0, ${bob})`}>

          {/* Body */}
          <path
            d="M270.548 382.714C175.869 479.647 86.1402 654.573 127.915 829.517C145.272 881.371 165.202 911.976 222.935 941.975C253.337 957.772 327.5 950.5 375.544 921.664L445.394 890.456C490.742 873.851 509.572 876.412 538.5 889.192C577.029 910.413 587.5 931.5 649.207 964.222C729.487 1006.79 793.127 956.041 817.514 889.192C874.808 742.915 814.514 422.978 650.331 310.479C516.054 226.594 403.003 247.226 270.548 382.714Z"
            fill="#F7D145"
            filter={`url(#${p("f0")})`}
          />

          {/* Left arm — drawn after body so it renders in front */}
          <g transform={`rotate(${leftSway}, 226, 578)`}>
            <path
              d="M257.7 773.068C271.729 736.698 272.987 728.506 287.23 709.133C299.638 692.255 259.842 627.746 226.232 577.586C219.862 568.08 205.393 568.903 199.906 578.945C176.511 621.76 137.044 694.31 143.077 742.936C156.218 848.842 228.429 842.851 257.7 773.068Z"
              fill="#F7D145"
              filter={`url(#${p("f4")})`}
            />
          </g>

          {/* Right arm */}
          <g transform={`rotate(${rightSway}, 712, 578)`}>
            <path
              d="M680.851 773.156C666.823 736.786 665.565 728.594 651.321 709.221C638.913 692.343 678.709 627.834 712.32 577.674C718.689 568.167 733.158 568.991 738.645 579.033C762.04 621.848 801.508 694.398 795.474 743.024C782.333 848.93 710.122 842.939 680.851 773.156Z"
              fill="#F7D145"
              filter={`url(#${p("f5")})`}
            />
          </g>

          {/* ── Head group: drift + squash ──────────────────────────────────── */}
          <g transform={
            `translate(${headDx}, ${headDy}) ` +
            `translate(493, 145) scale(${headSquashX}, ${headSquashY}) translate(-493, -145)`
          }>

            {/* Neck shadows */}
            <g opacity={0.4} filter={`url(#${p("f2")})`}>
              <path d="M450.376 270.172C464.042 264.005 502.076 255.372 544.876 270.172C598.376 288.672 415.876 288.172 450.376 270.172Z" fill="#B23C05"/>
            </g>
            <g opacity={0.4} filter={`url(#${p("f3")})`}>
              <path d="M533.5 245.499C524.956 248.602 489.943 257.335 463.186 249.888C429.74 240.578 555.068 236.442 533.5 245.499Z" fill="#B23C05"/>
            </g>

            {/* Head circle */}
            <circle cx={493} cy={145} r={110} fill="#F7D145" filter={`url(#${p("f1")})`}/>

            {/* ── Normal round eyes (fade out as heart phase begins) ────────── */}
            <g opacity={normalEyeOpacity}>
              {/* Left eye */}
              <g transform={`translate(411.48, 465) scale(1, ${eyeScaleNormal}) translate(-411.48, -465)`}>
                <path d="M411.48 428C419.679 428 423 432 424.408 434.321C431.456 442.807 434.448 450.812 435.286 461.939C436.531 478.451 428.581 501.025 409.176 501.922C402.907 502.212 396.783 499.978 392.177 495.714C372.967 478.168 379.456 428.811 411.48 428Z" fill="#1C170B"/>
                <g filter={`url(#${p("f6")})`}>
                  <path d="M402.589 435.31C405.113 435.115 406.119 435.015 408.226 436.218C409.449 437.699 409.295 438.305 409.367 440.116C410.18 440.625 410.898 441.111 411.694 441.647L411.904 442.956C419.014 456.194 406.034 468.295 397.004 457.028C387.109 457.791 393.027 445.603 396.045 441.344C398.038 438.531 399.869 437.302 402.589 435.31Z" fill="#FAF3EC"/>
                </g>
                <g filter={`url(#${p("f7")})`}>
                  <path d="M402.405 435.12C405.005 434.923 406.041 434.822 408.211 436.033C409.471 437.522 409.312 438.132 409.386 439.954C410.224 440.465 410.964 440.954 411.784 441.493L412 442.811C408.557 441.118 406.625 439.187 402.54 440.654C395.773 443.086 394.268 451.112 396.652 456.966C386.459 457.733 392.555 445.473 395.664 441.189C397.717 438.36 399.602 437.123 402.405 435.12Z" fill="#3A372F"/>
                </g>
              </g>

              {/* Right eye */}
              <g transform={`translate(589.37, 466) scale(1, ${eyeScaleNormal}) translate(-589.37, -466)`}>
                <path d="M589.37 428.706C621.867 428.523 630.994 493.598 594.352 502.663C555.686 504.419 554.456 433.119 589.37 428.706Z" fill="#1C170B"/>
                <g filter={`url(#${p("f8")})`}>
                  <path d="M576.491 452.759C577.097 454.049 577.14 454.759 576.609 455.979C569.334 454.164 573.452 439.586 580.007 437.664C584.2 436.436 587.824 438.013 589.306 442.115C592.619 444.137 594.847 446.01 595.749 450.049C596.355 452.791 595.845 455.661 594.331 458.027C589.038 466.354 580.303 462.46 578.515 452.619C577.656 451.775 577.93 451.624 577.758 450.079L577.591 450.499L577.887 450.8L577.387 452.615L576.491 452.759Z" fill="#FAF3EC"/>
                </g>
                <g filter={`url(#${p("f9")})`}>
                  <path d="M576.06 452.732C576.72 454.041 576.766 454.762 576.188 456C568.275 454.158 572.754 439.363 579.885 437.413C584.446 436.166 588.388 437.766 590 441.93L585.246 442.04C580.159 445.421 579.418 446.592 578.261 452.59C577.327 451.734 577.625 451.58 577.438 450.013L577.257 450.438L577.578 450.743L577.035 452.586L576.06 452.732Z" fill="#312E24"/>
                </g>
                <g filter={`url(#${p("f10")})`}>
                  <path d="M576.49 452.759L575.948 452.886L575.475 452.235C575.11 444.84 575.121 438.674 584.935 442.224C580.259 445.556 579.577 446.709 578.514 452.619C577.655 451.776 577.929 451.624 577.757 450.08L577.591 450.499L577.886 450.8L577.387 452.615L576.49 452.759Z" fill="#534639"/>
                </g>
              </g>
            </g>

            {/* ── Heart eyes (fade in, pulse with love) ─────────────────────── */}
            {/*
                Heart eye paths are in "love SVG space" (head cx=379.614 cy=114).
                translate(HX, HY) = translate(113.386, 31) maps them to this
                1000×1000 idelMascot space (head cx=493 cy=145).
                Pulse scale is applied around each heart's own centre.
                Left heart centre in love SVG: ~(312, 433)
                Right heart centre in love SVG: ~(470, 432)
            */}
            <g opacity={heartEyeOpacity}>

              {/* Soft pink glow behind the hearts */}
              <ellipse cx={425} cy={464} rx={38} ry={30} fill="#FF6B8B" opacity={0.18} filter={`url(#${p("glow")})`}/>
              <ellipse cx={584} cy={465} rx={38} ry={30} fill="#FF6B8B" opacity={0.18} filter={`url(#${p("glow")})`}/>

              {/* Left heart eye — scale around its centre (312,433) in love-SVG space */}
              <g transform={`translate(${HX}, ${HY}) translate(312, 433) scale(${heartBeat}) translate(-312, -433)`}>
                <path d="M309.528 412.554C316.357 407.26 320.889 400.095 331.012 401.094C339.939 401.812 347.398 414.839 345.579 422.994C342.606 436.319 328.949 446.655 319.814 456.123C318.474 457.512 314.226 461.046 312.55 461.737C313.245 461.996 313.3 461.654 314.345 462.028L314.024 462.208C309.672 461.395 309.368 460.135 305.68 457.217L305.402 457.454C305.572 459.556 308.83 461.362 310.613 463.457L310.539 463.96C305.339 458.843 286.396 445.795 285.105 441.973C286.363 443.268 287.409 444.42 288.884 445.485L289.268 445.154C288.823 443.956 286.606 442.228 285.561 441.008L285.791 440.321C283.919 437.892 282.17 435.78 280.526 433.063C272.519 419.82 281.925 402.881 297.772 404.673C301.678 405.116 306.342 407.944 308.673 411.248C308.972 411.676 309.257 412.111 309.528 412.554Z" fill="#DF6266"/>
                <path d="M312.55 461.736C313.245 461.995 313.3 461.653 314.345 462.027L314.023 462.207C309.671 461.394 309.367 460.134 305.68 457.216L305.402 457.453C305.572 459.555 308.829 461.361 310.613 463.456L310.539 463.96C305.339 458.842 286.396 445.794 285.105 441.972C286.363 443.268 287.408 444.419 288.884 445.484L289.267 445.153C288.823 443.955 286.605 442.228 285.56 441.008L285.791 440.32C294.825 448.827 303.117 454.01 312.55 461.736Z" fill="#F7D145"/>
                <path d="M320.235 405.907C322.834 405.918 329.975 406.908 331.182 409.254C329.977 412.892 325.408 411.266 322.545 411.046C321.563 410.97 321.286 411.546 320.603 412.172C318.832 413.875 315.912 416.685 313.35 415.893C311.056 412.468 317.628 407.429 320.235 405.907Z" fill="#F7CBCF"/>
                <path d="M294.67 413.177C296.271 413.159 296.778 413.115 298.207 413.839C300.033 416.095 298.468 417.502 296.97 419.543C295.925 419.733 294.792 419.183 293.908 418.708C292.179 416.779 293.496 414.922 294.67 413.177Z" fill="#F7CBCF"/>
              </g>

              {/* Right heart eye — scale around its centre (470,432) in love-SVG space */}
              <g transform={`translate(${HX}, ${HY}) translate(470, 432) scale(${heartBeat}) translate(-470, -432)`}>
                <path d="M443.479 441.876C435.348 431.202 429.644 416.448 442.673 406.948C455.28 397.754 460.826 407.185 469.392 413.799C475.761 410.114 481.06 404.044 489.173 406.213C493.482 407.365 498.274 411.374 500.184 415.271C511.834 439.058 477.213 454.036 464.376 463.741L464.404 464.011L463.938 464.177C461.818 463.029 458.23 457.264 454.601 455.317L454.167 455.659C454.995 459.211 459.324 463.162 461.881 466.278C455.136 460.531 448.861 450.362 442.42 443.884C441.966 443.427 442.162 442.984 442.319 442.466L443.479 441.876Z" fill="#DF6266"/>
                <path d="M464.374 463.741L464.402 464.011L463.937 464.177C461.817 463.029 458.229 457.264 454.599 455.317L454.166 455.659C454.994 459.211 459.323 463.162 461.879 466.278C455.135 460.531 448.86 450.362 442.419 443.884C441.965 443.427 442.16 442.984 442.318 442.466L443.478 441.876C450.036 449.329 457.637 455.932 464.374 463.741Z" fill="#F7D145"/>
                <path d="M451.02 410.51C453.805 410.226 455.531 410.662 458.212 411.216C456.494 413.134 455.392 413.839 453.216 415.21C451.742 415.908 450.139 416.232 448.523 415.955C447.409 415.181 447.696 415.617 447.415 414.347C448.034 412.223 449.21 411.648 451.02 410.51Z" fill="#FCF8F3"/>
              </g>

            </g>

            {/* ── Cheeks (get more flushed during heart phase) ─────────────── */}
            <g opacity={cheekOpacity}>
              <path d="M354.002 488.785C366.292 488.07 381.734 490.477 385.001 505.019C386.026 509.579 385.143 514.363 382.556 518.257C378.409 524.432 372.217 526.795 365.337 528.245C353.923 529.158 338.873 527.064 334.774 514.24C333.375 509.718 333.887 504.821 336.192 500.686C339.888 493.968 346.962 490.735 354.002 488.785Z" fill="#F9A6A0"/>
              <g filter={`url(#${p("f11")})`}>
                <path d="M368 494C373.244 494.048 380.363 498.673 380 504C375.832 504.091 367.526 498.087 368 494Z" fill="#FDC3BF"/>
              </g>
            </g>
            <g opacity={cheekOpacity}>
              <path d="M626.146 494.285C641.877 485.407 671.147 495.187 664.86 516.522C657.951 539.968 605.954 533.98 615.075 505.471C615.73 503.36 618.571 499.408 620.251 497.866C621.588 496.68 624.466 495.224 626.146 494.285Z" fill="#EF928B"/>
              <g filter={`url(#${p("f12")})`}>
                <path d="M632.013 497C626.77 497.048 619.65 501.673 620.013 507C624.181 507.091 632.487 501.087 632.013 497Z" fill="#FDC3BF"/>
              </g>
            </g>

            {/* ── Mouth — closed content smile (stays consistent) ───────────── */}
            <path d="M471.504 494.784C471.5 491.499 475 489 478.416 490.134C480.5 491.5 480.95 493.63 482.461 495.842C489.371 505.97 498.06 507.141 509.126 502.936C514.767 498.973 514.929 497.593 518.612 491.664C528.419 484.735 532.464 504.579 511.184 513.085C503.114 516.238 494.124 516.055 486.187 512.586C478.627 509.187 473.047 503.065 471.504 494.784Z" fill="#1C170B"/>
            <path d="M509.127 502.936C514.767 498.973 514.929 497.593 518.612 491.664L520.234 492.572C521.198 496.986 512.309 506.706 507.958 505.884L507.711 505.234L509.127 502.936Z" fill="#312E24"/>

          </g>
          {/* end head group */}

        </g>
        {/* end bob group */}

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
