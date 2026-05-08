import React from "react";
import {
  AbsoluteFill,
  Easing,
  interpolate,
  useCurrentFrame,
  useVideoConfig,
} from "remotion";

export const BlackMascotWink: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps, width, height } = useVideoConfig();
  const p = (k: string) => `bmwk-${k}`;

  // ── Relaxed body bob ─────────────────────────────────────────────────────
  const bob = Math.sin((frame / fps) * Math.PI * 1.0) * 10;

  // ── Head drift + squash ─────────────────────────────────────────────────
  const dotPhase = (frame / fps) * Math.PI;
  const headDx = Math.sin(dotPhase * 0.7) * 5;
  const headDy = Math.sin(dotPhase) * 7;
  const press = Math.max(0, Math.sin(dotPhase));
  const headSquashY = 1 - 0.07 * press;
  const headSquashX = 1 + 0.05 * press;

  // ── Wink transition: right eye open → wink ──────────────────────────────
  const winkProgress = interpolate(frame, [60, 78], [0, 1], {
    easing: Easing.inOut(Easing.quad),
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  const openRightEyeOpacity = 1 - winkProgress;
  const winkEyeOpacity = winkProgress;

  // Slight head tilt as wink comes in
  const headTilt = interpolate(frame, [60, 85], [0, 4], {
    easing: Easing.out(Easing.quad),
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });

  // ── Left eye blink — only after wink is set (frame 95+) ─────────────────
  const blinkPeriod = Math.round(fps * 3.5);
  const blinkDur = Math.round(fps * 0.14);
  const blinkOffset = frame < 95 ? 0 : (frame - 95) % blinkPeriod;
  const leftEyeScaleY =
    blinkOffset < blinkDur
      ? interpolate(
          blinkOffset,
          [0, blinkDur * 0.35, blinkDur * 0.65, blinkDur],
          [1, 0.06, 0.06, 1],
          { extrapolateLeft: "clamp", extrapolateRight: "clamp" }
        )
      : 1;

  // ── Right arm — waves only after wink is set ────────────────────────────
  const rightArmWave = interpolate(frame, [70, 95], [0, 1], {
    easing: Easing.out(Easing.quad),
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  const rightArmAngle =
    (10 + Math.sin((frame / fps) * Math.PI * 2.5) * 22) * rightArmWave;

  // ── Left arm — gentle idle sway ─────────────────────────────────────────
  const leftArmAngle = Math.sin((frame / fps) * Math.PI * 0.9) * 7;

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
          <radialGradient id={p("ground")} cx="0.5" cy="0.5" r="0.5">
            <stop offset="0%" stopColor="#000000" stopOpacity="0.28" />
            <stop offset="100%" stopColor="#000000" stopOpacity="0" />
          </radialGradient>

          {/* Body */}
          <filter id={p("f0")} x="90.3867" y="238.634" width="765.27" height="762.131" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
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
          <filter id={p("f2")} x="423.5" y="239.5" width="153.773" height="66.8594" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="11.25"/>
          </filter>
          <filter id={p("f3")} x="434.977" y="217.947" width="123.535" height="57.3701" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="11.25"/>
          </filter>

          {/* Left arm */}
          <filter id={p("f4")} x="138.461" y="555.812" width="155.094" height="272.386" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
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
          <filter id={p("f9")} x="645" y="555.9" width="155.094" height="272.386" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
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

          {/* Left eye highlights */}
          <filter id={p("f5")} x="390.216" y="433.891" width="25.0336" height="28.893" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.65"/>
          </filter>
          <filter id={p("f6")} x="390.3" y="434.3" width="22.4" height="23.4" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.35"/>
          </filter>

          {/* Right open eye highlights */}
          <filter id={p("f5r")} x="570.75" y="433.891" width="25.0336" height="28.893" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.65"/>
          </filter>
          <filter id={p("f6r")} x="573.3" y="434.3" width="22.4" height="23.4" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.35"/>
          </filter>

          {/* Cheek highlights */}
          <filter id={p("f7")} x="366.18" y="492.2" width="15.6352" height="13.601" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.9"/>
          </filter>
          <filter id={p("f8")} x="618.2" y="495.2" width="15.6352" height="13.601" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.9"/>
          </filter>
        </defs>

        {/* Ground shadow */}
        <ellipse cx={500} cy={978} rx={290} ry={22}
          fill={`url(#${p("ground")})`}
          transform={`scale(${1 - bob / 500}, 1)`}
          style={{ transformOrigin: "500px 978px" }}
        />

        {/* Everything bobs */}
        <g transform={`translate(0, ${bob})`}>

          {/* Body */}
          <path
            d="M270.549 382.714C175.87 479.647 86.1412 654.573 127.916 829.517C145.273 881.371 165.203 911.976 222.936 941.975C253.338 957.772 327.501 950.5 375.545 921.664L445.395 890.456C490.743 873.851 509.573 876.412 538.501 889.192C577.03 910.413 587.501 931.5 649.208 964.222C729.488 1006.79 793.127 956.041 817.515 889.192C874.809 742.915 814.515 422.978 650.332 310.479C516.055 226.594 403.004 247.226 270.549 382.714Z"
            fill="#3A3A3A"
            filter={`url(#${p("f0")})`}
          />

          {/* Left arm */}
          <g transform={`rotate(${leftArmAngle}, 226, 578)`}>
            <path
              d="M257.703 773.068C271.731 736.698 272.99 728.506 287.233 709.133C299.641 692.255 259.845 627.746 226.234 577.586C219.865 568.08 205.396 568.903 199.909 578.945C176.514 621.76 137.047 694.31 143.08 742.936C156.221 848.842 228.432 842.851 257.703 773.068Z"
              fill="#3A3A3A"
              filter={`url(#${p("f4")})`}
            />
          </g>

          {/* Right arm — waves after wink */}
          <g transform={`rotate(${rightArmAngle}, 712, 577)`}>
            <path
              d="M680.852 773.156C666.823 736.786 665.565 728.594 651.322 709.221C638.913 692.343 678.709 627.834 712.32 577.674C718.69 568.167 733.159 568.991 738.646 579.033C762.04 621.848 801.508 694.398 795.474 743.024C782.334 848.93 710.122 842.939 680.852 773.156Z"
              fill="#3A3A3A"
              filter={`url(#${p("f9")})`}
            />
          </g>

          {/* Head group: drift + tilt + squash */}
          <g transform={`translate(${headDx}, ${headDy}) rotate(${headTilt}, 493, 465)`}>
            <g transform={`translate(493, 145) scale(${headSquashX}, ${headSquashY}) translate(-493, -145)`}>

              {/* Neck shadows */}
              <g opacity={0.4} filter={`url(#${p("f2")})`}>
                <path d="M450.376 270.172C464.042 264.005 502.076 255.372 544.876 270.172C598.376 288.672 415.876 288.172 450.376 270.172Z" fill="#030100"/>
              </g>
              <g opacity={0.4} filter={`url(#${p("f3")})`}>
                <path d="M533.499 245.499C524.955 248.602 489.943 257.335 463.185 249.888C429.739 240.578 555.068 236.442 533.499 245.499Z" fill="white"/>
              </g>

              {/* Head circle */}
              <circle cx={493} cy={145} r={110} fill="#3A3A3A" filter={`url(#${p("f1")})`}/>

              {/* Left eye — blinks periodically after wink */}
              <g transform={`translate(411, 465) scale(1, ${leftEyeScaleY}) translate(-411, -465)`}>
                <path d="M411.479 428C419.678 428 423 432 424.408 434.321C431.455 442.807 434.448 450.812 435.286 461.939C436.53 478.451 428.58 501.025 409.175 501.922C402.907 502.212 396.782 499.978 392.176 495.714C372.967 478.168 379.456 428.811 411.479 428Z" fill="#1C170B"/>
                <g filter={`url(#${p("f5")})`}>
                  <path d="M402.588 435.31C405.111 435.115 406.117 435.015 408.224 436.218C409.447 437.699 409.293 438.305 409.365 440.116C410.178 440.625 410.896 441.111 411.693 441.647L411.902 442.956C419.012 456.194 406.032 468.295 397.002 457.028C387.107 457.791 393.025 445.603 396.043 441.344C398.036 438.531 399.867 437.302 402.588 435.31Z" fill="#FAF3EC"/>
                </g>
                <g filter={`url(#${p("f6")})`}>
                  <path d="M402.405 435.12C405.005 434.923 406.041 434.822 408.211 436.033C409.471 437.522 409.312 438.132 409.386 439.954C410.224 440.465 410.964 440.954 411.784 441.493L412 442.811C408.557 441.118 406.625 439.187 402.54 440.654C395.773 443.086 394.268 451.112 396.652 456.966C386.459 457.733 392.555 445.473 395.664 441.189C397.717 438.36 399.602 437.123 402.405 435.12Z" fill="#3A372F"/>
                </g>
              </g>

              {/* Right eye: open (fades out) → wink (fades in) */}
              <g opacity={openRightEyeOpacity}>
                <path d="M574.521 428C566.322 428 563 432 561.592 434.321C554.545 442.807 551.552 450.812 550.714 461.939C549.47 478.451 557.42 501.025 576.825 501.922C583.093 502.212 589.218 499.978 593.824 495.714C613.033 478.168 606.544 428.811 574.521 428Z" fill="#1C170B"/>
                <g filter={`url(#${p("f5r")})`}>
                  <path d="M583.412 435.31C580.889 435.115 579.883 435.015 577.776 436.218C576.553 437.699 576.707 438.305 576.635 440.116C575.822 440.625 575.104 441.111 574.307 441.647L574.098 442.956C566.988 456.194 579.968 468.295 588.998 457.028C598.893 457.791 592.975 445.603 589.957 441.344C587.964 438.531 586.133 437.302 583.412 435.31Z" fill="#FAF3EC"/>
                </g>
                <g filter={`url(#${p("f6r")})`}>
                  <path d="M583.595 435.12C580.995 434.923 579.959 434.822 577.789 436.033C576.529 437.522 576.688 438.132 576.614 439.954C575.776 440.465 575.036 440.954 574.216 441.493L574 442.811C577.443 441.118 579.375 439.187 583.46 440.654C590.227 443.086 591.732 451.112 589.348 456.966C599.541 457.733 593.445 445.473 590.336 441.189C588.283 438.36 586.398 437.302 583.595 435.12Z" fill="#3A372F"/>
                </g>
              </g>

              {/* Wink right eye — stroke arch */}
              <g opacity={winkEyeOpacity}>
                <path
                  d="M620.887 447.7L570.836 463.683C568.007 464.586 568.069 468.61 570.924 469.425L620.887 483.7"
                  stroke="black"
                  strokeWidth="7"
                  strokeLinecap="round"
                  fill="none"
                />
              </g>

              {/* Left cheek */}
              <path d="M354.002 488.785C366.292 488.07 381.734 490.477 385.001 505.019C386.026 509.579 385.143 514.363 382.556 518.257C378.409 524.432 372.217 526.795 365.337 528.245C353.923 529.158 338.873 527.064 334.774 514.24C333.375 509.718 333.887 504.821 336.192 500.686C339.888 493.968 346.962 490.735 354.002 488.785Z" fill="#F9A6A0"/>
              <g filter={`url(#${p("f7")})`}>
                <path d="M368 494C373.243 494.048 380.362 498.673 380 504C375.832 504.091 367.526 498.087 368 494Z" fill="#FDC3BF"/>
              </g>

              {/* Right cheek */}
              <path d="M626.148 494.285C641.879 485.407 671.15 495.187 664.863 516.522C657.953 539.968 605.956 533.98 615.078 505.471C615.733 503.36 618.573 499.408 620.253 497.867C621.59 496.68 624.468 495.224 626.148 494.285Z" fill="#EF928B"/>
              <g filter={`url(#${p("f8")})`}>
                <path d="M632.016 497C626.773 497.048 619.653 501.673 620.016 507C624.184 507.091 632.49 501.087 632.016 497Z" fill="#FDC3BF"/>
              </g>

              {/* Smirk mouth */}
              <path d="M529.372 496.072C531.605 520.248 511.988 530.895 498.11 530.326C478.46 529.52 465.731 508.164 469.081 496.075C472.43 488.009 486.945 493.048 495.877 494.06C499.571 494.06 503.226 493.368 506.814 492.493C514.924 490.516 527.623 488.965 529.372 496.072Z" fill="#03050D"/>
              <path d="M518.002 516.476C508.038 503.918 489.302 508.496 481.546 516.842C479.83 518.689 480.226 521.523 482.178 523.117C492.266 531.35 506.183 531.37 517.046 523.176C519.173 521.572 519.658 518.563 518.002 516.476Z" fill="#E06B51"/>

            </g>
          </g>
          {/* end head group */}

        </g>
        {/* end bob group */}

      </svg>
    </AbsoluteFill>
  );
};
