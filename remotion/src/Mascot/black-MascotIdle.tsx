import React from "react";
import { AbsoluteFill, useCurrentFrame, useVideoConfig } from "remotion";

/**
 * Black idle mascot — uses exact paths and filters from BlackIdelmascot.svg
 * with the same bob, head-drift, arm-sway, and blink animations as the yellow idle.
 */
export const BlackMascotIdle: React.FC = () => {
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

  // Blink every ~2.6s for ~6 frames.
  const blinkPeriod = Math.round(fps * 2.6);
  const blinkOffset = Math.round(blinkPeriod / 2);
  const inBlink = (frame + blinkOffset) % blinkPeriod < 6;
  const eyeScale = inBlink ? 0.12 : 1;

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
          <radialGradient id="bmi-ground" cx="0.5" cy="0.5" r="0.5">
            <stop offset="0%" stopColor="#000000" stopOpacity="0.35" />
            <stop offset="100%" stopColor="#000000" stopOpacity="0" />
          </radialGradient>

          {/* Body filter — from BlackIdelmascot.svg filter0_iig */}
          <filter id="bmi-f0" x="90.3867" y="238.634" width="765.266" height="762.131" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
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
          <filter id="bmi-f1" x="379.002" y="22" width="233" height="237" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
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
          <filter id="bmi-f2" x="423.502" y="239.5" width="153.77" height="66.86" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feGaussianBlur stdDeviation="11.25" />
          </filter>
          <filter id="bmi-f3" x="434.979" y="217.947" width="123.535" height="57.3708" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feGaussianBlur stdDeviation="11.25" />
          </filter>

          {/* Left arm filter — from BlackIdelmascot.svg filter4_iig */}
          <filter id="bmi-f4" x="138.459" y="555.812" width="155.094" height="272.386" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
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
          <filter id="bmi-f5" x="645" y="555.9" width="155.094" height="272.386" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
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

          {/* Left eye highlight filters */}
          <filter id="bmi-f6" x="390.22" y="433.891" width="25.0336" height="28.893" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feGaussianBlur stdDeviation="0.65" />
          </filter>
          <filter id="bmi-f7" x="390.302" y="434.3" width="22.4" height="23.4" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feGaussianBlur stdDeviation="0.35" />
          </filter>

          {/* Right eye highlight filters */}
          <filter id="bmi-f8" x="570.86" y="435.358" width="27.0383" height="29.1121" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feGaussianBlur stdDeviation="0.95" />
          </filter>
          <filter id="bmi-f9" x="571.302" y="436.3" width="19.4" height="20.4" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feGaussianBlur stdDeviation="0.35" />
          </filter>
          <filter id="bmi-f10" x="574.669" y="440.492" width="10.9664" height="13.0938" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feGaussianBlur stdDeviation="0.35" />
          </filter>

          {/* Cheek highlight filters */}
          <filter id="bmi-f11" x="366.18" y="492.2" width="15.6312" height="13.601" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feGaussianBlur stdDeviation="0.9" />
          </filter>
          <filter id="bmi-f12" x="618.202" y="495.2" width="15.6312" height="13.601" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix" />
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape" />
            <feGaussianBlur stdDeviation="0.9" />
          </filter>
        </defs>

        {/* Ground shadow */}
        <g transform={`translate(500, 975) scale(${1 - bob / 600}, 1)`}>
          <ellipse cx={0} cy={0} rx={300} ry={28} fill="url(#bmi-ground)" />
        </g>

        {/* Everything bobs together */}
        <g transform={`translate(0, ${bob})`}>

          {/* Head — drifts + squashes independently */}
          <g transform={
            `translate(${dotDx}, ${dotDy}) ` +
            `translate(493 145) scale(${dotSquashX} ${dotSquashY}) translate(-493 -145)`
          }>
            <g filter="url(#bmi-f1)">
              <circle cx="493.002" cy="145" r="110" fill="#3A3A3A" />
            </g>
          </g>

          {/* Body */}
          <g filter="url(#bmi-f0)">
            <path d="M270.549 382.715C175.87 479.648 86.1412 654.573 127.916 829.517C145.273 881.371 165.203 911.977 222.936 941.975C253.338 957.772 327.501 950.5 375.545 921.664L445.395 890.457C490.743 873.851 509.573 876.412 538.501 889.192C577.03 910.414 587.501 931.5 649.208 964.222C729.488 1006.79 793.127 956.041 817.515 889.192C874.809 742.915 814.515 422.979 650.332 310.48C516.055 226.594 403.004 247.226 270.549 382.715Z" fill="#3A3A3A" />
          </g>

          {/* Right steady arm — gentle sway */}
          <g transform={`rotate(${steadySway}, 655, 709)`}>
            <g filter="url(#bmi-f5)">
              <path d="M680.852 773.156C666.823 736.786 665.565 728.594 651.322 709.221C638.913 692.343 678.709 627.834 712.32 577.674C718.69 568.167 733.159 568.991 738.646 579.033C762.04 621.848 801.508 694.398 795.474 743.024C782.334 848.93 710.122 842.939 680.852 773.156Z" fill="#3A3A3A" />
            </g>
          </g>

          {/* Left arm — gentle sway */}
          <g transform={`rotate(${leftSway}, 290, 700)`}>
            <g filter="url(#bmi-f4)">
              <path d="M257.701 773.068C271.729 736.698 272.988 728.506 287.231 709.133C299.639 692.255 259.843 627.746 226.233 577.586C219.863 568.08 205.394 568.903 199.907 578.945C176.512 621.76 137.045 694.31 143.078 742.936C156.219 848.842 228.43 842.851 257.701 773.068Z" fill="#3A3A3A" />
            </g>
          </g>

          {/* Neck shadows */}
          <g opacity={0.4} filter="url(#bmi-f2)">
            <path d="M450.377 270.172C464.044 264.005 502.077 255.372 544.877 270.172C598.377 288.672 415.877 288.172 450.377 270.172Z" fill="#030100" />
          </g>
          <g opacity={0.4} filter="url(#bmi-f3)">
            <path d="M533.501 245.499C524.957 248.602 489.945 257.335 463.187 249.888C429.741 240.578 555.07 236.442 533.501 245.499Z" fill="white" />
          </g>

          {/* Left eye — scaleY collapses on blink */}
          <g transform={`translate(411, 465) scale(1, ${eyeScale}) translate(-411, -465)`}>
            <path d="M411.481 428C419.68 428 423.001 432 424.41 434.321C431.457 442.807 434.45 450.812 435.288 461.939C436.532 478.451 428.582 501.025 409.177 501.922C402.908 502.212 396.784 499.978 392.178 495.714C372.969 478.168 379.457 428.811 411.481 428Z" fill="#1C170B" />
            {!inBlink && (
              <>
                <g filter="url(#bmi-f6)">
                  <path d="M402.591 435.31C405.115 435.115 406.121 435.015 408.228 436.218C409.451 437.699 409.297 438.305 409.369 440.116C410.182 440.625 410.9 441.111 411.696 441.647L411.906 442.956C419.016 456.194 406.036 468.295 397.006 457.028C387.111 457.791 393.029 445.603 396.047 441.344C398.04 438.531 399.871 437.302 402.591 435.31Z" fill="#FAF3EC" />
                </g>
                <g filter="url(#bmi-f7)">
                  <path d="M402.407 435.12C405.007 434.923 406.043 434.822 408.213 436.033C409.473 437.522 409.314 438.132 409.388 439.954C410.226 440.465 410.966 440.954 411.786 441.493L412.002 442.811C408.559 441.118 406.627 439.187 402.542 440.654C395.775 443.086 394.27 451.112 396.654 456.966C386.461 457.733 392.557 445.473 395.666 441.189C397.719 438.36 399.604 437.123 402.407 435.12Z" fill="#3A372F" />
                </g>
              </>
            )}
          </g>

          {/* Right eye — scaleY collapses on blink */}
          <g transform={`translate(589, 465) scale(1, ${eyeScale}) translate(-589, -465)`}>
            <path d="M589.371 428.706C621.869 428.523 630.996 493.598 594.353 502.663C555.687 504.419 554.458 433.119 589.371 428.706Z" fill="#1C170B" />
            {!inBlink && (
              <>
                <g filter="url(#bmi-f8)">
                  <path d="M576.492 452.759C577.098 454.049 577.141 454.759 576.61 455.979C569.335 454.164 573.453 439.586 580.008 437.664C584.201 436.436 587.825 438.013 589.307 442.115C592.62 444.137 594.848 446.01 595.75 450.049C596.356 452.791 595.846 455.661 594.332 458.027C589.039 466.354 580.304 462.46 578.516 452.619C577.657 451.775 577.931 451.624 577.759 450.079L577.592 450.499L577.888 450.8L577.388 452.615L576.492 452.759Z" fill="#FAF3EC" />
                </g>
                <g filter="url(#bmi-f9)">
                  <path d="M576.062 452.732C576.721 454.041 576.768 454.762 576.19 456C568.277 454.158 572.756 439.363 579.887 437.413C584.448 436.166 588.39 437.766 590.002 441.93L585.248 442.04C580.161 445.421 579.42 446.592 578.263 452.59C577.329 451.734 577.627 451.58 577.44 450.013L577.259 450.438L577.58 450.743L577.037 452.586L576.062 452.732Z" fill="#312E24" />
                </g>
                <g filter="url(#bmi-f10)">
                  <path d="M576.492 452.759L575.949 452.886L575.477 452.235C575.112 444.84 575.123 438.674 584.937 442.224C580.261 445.556 579.579 446.709 578.516 452.619C577.657 451.776 577.93 451.624 577.759 450.08L577.592 450.499L577.887 450.8L577.388 452.615L576.492 452.759Z" fill="#534639" />
                </g>
              </>
            )}
          </g>

          {/* Left cheek */}
          <path d="M354.002 488.785C366.292 488.07 381.734 490.477 385.001 505.019C386.026 509.579 385.143 514.363 382.556 518.257C378.409 524.432 372.217 526.795 365.337 528.245C353.923 529.158 338.873 527.064 334.774 514.24C333.375 509.718 333.887 504.821 336.192 500.686C339.888 493.968 346.962 490.735 354.002 488.785Z" fill="#F9A6A0" />
          <g filter="url(#bmi-f11)">
            <path d="M368 494C373.243 494.048 380.362 498.673 380 504C375.832 504.091 367.526 498.087 368 494Z" fill="#FDC3BF" />
          </g>

          {/* Right cheek */}
          <path d="M626.146 494.285C641.877 485.407 671.148 495.187 664.861 516.522C657.951 539.968 605.954 533.98 615.076 505.471C615.731 503.36 618.571 499.408 620.251 497.867C621.588 496.68 624.466 495.224 626.146 494.285Z" fill="#EF928B" />
          <g filter="url(#bmi-f12)">
            <path d="M632.014 497C626.771 497.048 619.651 501.673 620.014 507C624.182 507.091 632.488 501.087 632.014 497Z" fill="#FDC3BF" />
          </g>

          {/* Mouth */}
          <path d="M471.506 494.784C471.501 491.499 475.001 489 478.417 490.134C480.501 491.5 480.951 493.63 482.462 495.842C489.372 505.97 498.062 507.141 509.128 502.936C514.768 498.973 514.93 497.593 518.613 491.664C528.42 484.735 532.465 504.579 511.186 513.085C503.116 516.238 494.125 516.055 486.188 512.586C478.628 509.187 473.049 503.065 471.506 494.784Z" fill="#1C170B" />
          <path d="M509.129 502.936C514.769 498.973 514.931 497.593 518.614 491.664L520.236 492.572C521.2 496.986 512.311 506.706 507.96 505.884L507.713 505.234L509.129 502.936Z" fill="#312E24" />
        </g>
      </svg>
    </AbsoluteFill>
  );
};
