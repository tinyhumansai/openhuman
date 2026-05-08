import React from "react";
import {
  AbsoluteFill,
  Easing,
  interpolate,
  useCurrentFrame,
  useVideoConfig,
} from "remotion";

export const BlackMascotCrying: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps, width, height } = useVideoConfig();
  const p = (k: string) => `bmcry-${k}`;

  // ── Cry transition ─────────────────────────────────────────────────────────
  const cryProgress = interpolate(frame, [60, 90], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
    easing: Easing.inOut(Easing.cubic),
  });
  const normalFaceOpacity = 1 - cryProgress;
  const cryFaceOpacity = cryProgress;

  // ── Body bob — calm idle blends into faster sob shudder ───────────────────
  const idleBob = Math.sin((frame / fps) * Math.PI * 1.2) * 14;
  const sobBob =
    Math.sin((frame / fps) * Math.PI * 2.8) * 10 +
    Math.sin((frame / fps) * Math.PI * 5.5 + 0.7) * 3;
  const bob = idleBob * (1 - cryProgress) + sobBob * cryProgress;

  // ── Head drift + squash ───────────────────────────────────────────────────
  const dotPhase = (frame / fps) * Math.PI;
  const driftScale = 1 - cryProgress * 0.65;
  const headDx = Math.sin(dotPhase * 0.7) * 6 * driftScale;
  const headDy = Math.sin(dotPhase) * 9 * driftScale;
  const press = Math.max(0, Math.sin(dotPhase)) * driftScale;
  const headSquashY = 1 - 0.08 * press;
  const headSquashX = 1 + 0.05 * press;

  // ── Arms — gentle idle sway, droop down when crying ──────────────────────
  const leftSway = Math.sin((frame / fps) * Math.PI * 1.3) * 7;
  const rightSway = Math.sin((frame / fps) * Math.PI * 1.3 + 1.0) * 6;
  const leftArmAngle = leftSway + cryProgress * 14;
  const rightArmAngle = rightSway + cryProgress * 14;

  // ── Blink — only during idle phase ───────────────────────────────────────
  const blinkPeriod = Math.round(fps * 2.6);
  const blinkOffset = Math.round(blinkPeriod / 2);
  const inBlink = cryProgress < 0.15 && (frame + blinkOffset) % blinkPeriod < 6;
  const eyeScaleNormal = inBlink ? 0.12 : 1;

  // ── Cheeks — flush more as crying intensifies ─────────────────────────────
  const cheekOpacity =
    0.82 + cryProgress * 0.16 + Math.sin((frame / fps) * Math.PI * 1.1) * 0.05;

  // ── Tears ─────────────────────────────────────────────────────────────────
  const tearPeriod = Math.round(fps * 1.6);
  const getTear = (delayFrames: number, eyeX: number, eyeStartY: number) => {
    const startAt = 90 + delayFrames;
    if (frame < startAt) return { x: eyeX, y: eyeStartY, opacity: 0 };
    const cycleFrame = (frame - startAt) % tearPeriod;
    const t = cycleFrame / tearPeriod;
    const y = eyeStartY + t * 170;
    const opacity =
      interpolate(t, [0, 0.07, 0.68, 1.0], [0, 0.9, 0.75, 0], {
        extrapolateLeft: "clamp",
        extrapolateRight: "clamp",
      }) * cryProgress;
    return { x: eyeX, y, opacity };
  };

  const tL1 = getTear(0, 395, 485);
  const tL2 = getTear(Math.round(fps * 0.55), 408, 485);
  const tR1 = getTear(Math.round(fps * 0.22), 592, 485);
  const tR2 = getTear(Math.round(fps * 0.80), 603, 485);

  const size = Math.min(width, height) * 0.82;

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
            <feColorMatrix type="matrix" values="0 0 0 0 0.439078 0 0 0 0 0.439216 0 0 0 0 0.439216 0 0 0 1 0"/>
            <feBlend mode="normal" in2="shape" result="e1"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="-27" dy="-22"/><feGaussianBlur stdDeviation="29.75"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.0229492 0 0 0 0 0.0235294 0 0 0 0 0.0235294 0 0 0 1 0"/>
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
            <feColorMatrix type="matrix" values="0 0 0 0 0.0229492 0 0 0 0 0.0235294 0 0 0 0 0.0235294 0 0 0 1 0"/>
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
            <feColorMatrix type="matrix" values="0 0 0 0 0.439078 0 0 0 0 0.439216 0 0 0 0 0.439216 0 0 0 1 0"/>
            <feBlend mode="normal" in2="shape" result="e1"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="3" dy="-8"/><feGaussianBlur stdDeviation="3.55"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.0229492 0 0 0 0 0.0235294 0 0 0 0 0.0235294 0 0 0 0.8 0"/>
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
            <feColorMatrix type="matrix" values="0 0 0 0 0.439078 0 0 0 0 0.439216 0 0 0 0 0.439216 0 0 0 1 0"/>
            <feBlend mode="normal" in2="shape" result="e1"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dy="-8"/><feGaussianBlur stdDeviation="3.55"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.0229492 0 0 0 0 0.0235294 0 0 0 0 0.0235294 0 0 0 0.8 0"/>
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
        </defs>

        {/* Ground shadow */}
        <ellipse
          cx={500} cy={978} rx={290} ry={22}
          fill={`url(#${p("ground")})`}
          transform={`scale(${1 - bob / 500}, 1)`}
          style={{ transformOrigin: "500px 978px" }}
        />

        {/* Everything bobs together */}
        <g transform={`translate(0, ${bob})`}>

          {/* Body */}
          <path
            d="M270.548 382.714C175.869 479.647 86.1402 654.573 127.915 829.517C145.272 881.371 165.202 911.976 222.935 941.975C253.337 957.772 327.5 950.5 375.544 921.664L445.394 890.456C490.742 873.851 509.572 876.412 538.5 889.192C577.029 910.413 587.5 931.5 649.207 964.222C729.487 1006.79 793.127 956.041 817.514 889.192C874.808 742.915 814.514 422.978 650.331 310.479C516.054 226.594 403.003 247.226 270.548 382.714Z"
            fill="#3A3A3A"
            filter={`url(#${p("f0")})`}
          />

          {/* Left arm */}
          <g transform={`rotate(${leftArmAngle}, 226, 578)`}>
            <path
              d="M257.7 773.068C271.729 736.698 272.987 728.506 287.23 709.133C299.638 692.255 259.842 627.746 226.232 577.586C219.862 568.08 205.393 568.903 199.906 578.945C176.511 621.76 137.044 694.31 143.077 742.936C156.218 848.842 228.429 842.851 257.7 773.068Z"
              fill="#3A3A3A"
              filter={`url(#${p("f4")})`}
            />
          </g>

          {/* Right arm */}
          <g transform={`rotate(${rightArmAngle}, 712, 578)`}>
            <path
              d="M680.851 773.156C666.823 736.786 665.565 728.594 651.321 709.221C638.913 692.343 678.709 627.834 712.32 577.674C718.689 568.167 733.158 568.991 738.645 579.033C762.04 621.848 801.508 694.398 795.474 743.024C782.333 848.93 710.122 842.939 680.851 773.156Z"
              fill="#3A3A3A"
              filter={`url(#${p("f5")})`}
            />
          </g>

          {/* Tears */}
          {[tL1, tL2, tR1, tR2].map((t, i) => (
            <g key={i} transform={`translate(${t.x}, ${t.y})`} opacity={t.opacity}>
              <path
                d="M-5,0 A5,5,0,0,1,5,0 Q5,10 0,18 Q-5,10 -5,0 Z"
                fill="#7EC8F0"
              />
            </g>
          ))}

          {/* Head group: drift + squash */}
          <g transform={
            `translate(${headDx}, ${headDy}) ` +
            `translate(493, 145) scale(${headSquashX}, ${headSquashY}) translate(-493, -145)`
          }>

            {/* Neck shadows */}
            <g opacity={0.4} filter={`url(#${p("f2")})`}>
              <path d="M450.376 270.172C464.042 264.005 502.076 255.372 544.876 270.172C598.376 288.672 415.876 288.172 450.376 270.172Z" fill="#030100"/>
            </g>
            <g opacity={0.4} filter={`url(#${p("f3")})`}>
              <path d="M533.5 245.499C524.956 248.602 489.943 257.335 463.186 249.888C429.74 240.578 555.068 236.442 533.5 245.499Z" fill="white"/>
            </g>

            {/* Head circle */}
            <circle cx={493} cy={145} r={110} fill="#3A3A3A" filter={`url(#${p("f1")})`}/>

            {/* Normal eyes */}
            <g opacity={normalFaceOpacity}>
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
              <g transform={`translate(589.37, 465) scale(1, ${eyeScaleNormal}) translate(-589.37, -465)`}>
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

            {/* Crying eyes */}
            <g opacity={cryFaceOpacity}>
              <path
                d="M378.656 446.974L428.707 462.956C431.536 463.859 431.474 467.883 428.619 468.699L378.656 482.974"
                stroke="#1C170B"
                strokeWidth="7"
                strokeLinecap="round"
                fill="none"
              />
              <path
                d="M620.887 447.7L570.836 463.683C568.007 464.586 568.069 468.61 570.924 469.425L620.887 483.7"
                stroke="#1C170B"
                strokeWidth="7"
                strokeLinecap="round"
                fill="none"
              />
            </g>

            {/* Cheeks */}
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

            {/* Normal smile */}
            <g opacity={normalFaceOpacity}>
              <path d="M471.504 494.784C471.5 491.499 475 489 478.416 490.134C480.5 491.5 480.95 493.63 482.461 495.842C489.371 505.97 498.06 507.141 509.126 502.936C514.767 498.973 514.929 497.593 518.612 491.664C528.419 484.735 532.464 504.579 511.184 513.085C503.114 516.238 494.124 516.055 486.187 512.586C478.627 509.187 473.047 503.065 471.504 494.784Z" fill="#1C170B"/>
              <path d="M509.127 502.936C514.767 498.973 514.929 497.593 518.612 491.664L520.234 492.572C521.198 496.986 512.309 506.706 507.958 505.884L507.711 505.234L509.127 502.936Z" fill="#312E24"/>
            </g>

            {/* Sad frown */}
            <g opacity={cryFaceOpacity}>
              <path d="M524.086 523.541C524.09 526.826 520.59 529.325 517.175 528.191C515.09 526.825 514.641 524.696 513.13 522.483C506.22 512.355 497.53 511.184 486.464 515.389C480.823 519.352 480.661 520.732 476.978 526.661C467.172 533.591 463.127 513.746 484.406 505.24C492.476 502.088 501.467 502.27 509.404 505.739C516.964 509.138 522.543 515.26 524.086 523.541Z" fill="#1C170B"/>
              <path d="M486.463 515.389C480.823 519.352 480.661 520.733 476.978 526.661L475.356 525.754C474.392 521.339 483.281 511.62 487.631 512.441L487.879 513.091L486.463 515.389Z" fill="#312E24"/>
            </g>

          </g>
          {/* end head group */}

        </g>
        {/* end bob group */}

      </svg>
    </AbsoluteFill>
  );
};
