import React from "react";
import {
  AbsoluteFill,
  interpolate,
  useCurrentFrame,
  useVideoConfig,
} from "remotion";

/**
 * NewMascotBookReading — Bookreading.svg brought to life.
 *
 * • Book gently sways left-right as the character reads
 * • Both arms move in sync with the book sway
 * • Head leans slightly forward (toward book), slow nod
 * • Both eyes open with periodic blink
 * • Calm slow body bob (focused/relaxed reading)
 */
export const NewMascotBookReading: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps, width, height } = useVideoConfig();
  const p = (k: string) => `nmbr-${k}`;

  // ── Slow calm body bob — 0.7 Hz ─────────────────────────────────────────
  const bob = Math.sin((frame / fps) * Math.PI * 0.7) * 8;

  // ── Head drift — leans slightly toward book (+8 downward) ───────────────
  const dotPhase = (frame / fps) * Math.PI;
  const headDx = Math.sin(dotPhase * 0.6) * 4;
  const headDy = Math.sin(dotPhase * 0.7) * 6 + 8;
  const press = Math.max(0, Math.sin(dotPhase * 0.7));
  const headSquashY = 1 - 0.05 * press;
  const headSquashX = 1 + 0.035 * press;

  // ── Slow head nod — engaged in reading ──────────────────────────────────
  const headNod = Math.sin((frame / fps) * Math.PI * 0.35) * 2.5;

  // ── Book sway — gentle 0.4 Hz rock around book center ───────────────────
  const bookSway = Math.sin((frame / fps) * Math.PI * 0.4) * 2.8;

  // ── Left arm moves with book (pivot at left shoulder ~313, 640) ─────────
  const leftArmSway = bookSway * 0.75;

  // ── Right arm moves with book (pivot at right shoulder ~553, 682) ────────
  const rightArmSway = bookSway * 0.6;

  // ── Both eyes blink together every ~4s ──────────────────────────────────
  const blinkPeriod = Math.round(fps * 4.0);
  const blinkDur = Math.round(fps * 0.12);
  const blinkPhase = frame % blinkPeriod;
  const eyeScaleY =
    blinkPhase < blinkDur
      ? interpolate(
          blinkPhase,
          [0, blinkDur * 0.35, blinkDur * 0.65, blinkDur],
          [1, 0.06, 0.06, 1],
          { extrapolateLeft: "clamp", extrapolateRight: "clamp" }
        )
      : 1;

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

          {/* Left arm (book-holding) */}
          <filter id={p("f4")} x="185.258" y="624.721" width="260.633" height="160.296" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
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

          {/* Right arm (book-holding) */}
          <filter id={p("f5")} x="514.094" y="615.765" width="249.91" height="175.404" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
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

          {/* Left eye highlight */}
          <filter id={p("f6")} x="390.216" y="433.891" width="25.0336" height="28.893" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.65"/>
          </filter>
          <filter id={p("f7")} x="390.3" y="434.3" width="22.4" height="23.4" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.35"/>
          </filter>

          {/* Right eye highlight */}
          <filter id={p("f8")} x="570.858" y="435.358" width="27.0422" height="29.1125" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.95"/>
          </filter>
          <filter id={p("f9")} x="571.3" y="436.3" width="19.4" height="20.4" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.35"/>
          </filter>
          <filter id={p("f10")} x="574.667" y="440.492" width="10.9703" height="13.0943" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.35"/>
          </filter>

          {/* Cheek highlights */}
          <filter id={p("f11")} x="366.18" y="492.2" width="15.6352" height="13.601" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.9"/>
          </filter>
          <filter id={p("f12")} x="618.2" y="495.2" width="15.6352" height="13.601" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
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

        {/* ── Everything bobs ───────────────────────────────────────────────── */}
        <g transform={`translate(0, ${bob})`}>

          {/* ── Body ────────────────────────────────────────────────────────── */}
          <path
            d="M270.549 382.714C175.87 479.647 86.1412 654.573 127.916 829.517C145.273 881.371 165.203 911.976 222.936 941.975C253.338 957.772 327.501 950.5 375.545 921.664L445.395 890.456C490.743 873.851 509.573 876.412 538.501 889.192C577.03 910.413 587.501 931.5 649.208 964.222C729.488 1006.79 793.127 956.041 817.515 889.192C874.809 742.915 814.515 422.978 650.332 310.479C516.055 226.594 403.004 247.226 270.549 382.714Z"
            fill="#F7D145"
            filter={`url(#${p("f0")})`}
          />

          {/* ── Left arm + book + right arm sway together ────────────────────── */}
          <g transform={`rotate(${bookSway}, 480, 665)`}>

            {/* Left arm */}
            <g transform={`rotate(${leftArmSway}, 313, 640)`}>
              <path
                d="M380.49 658.503C342.194 651.221 333.909 651.451 312.297 640.91C293.469 631.728 237.137 682.441 193.813 724.498C185.602 732.469 189.006 746.556 199.868 750.154C246.183 765.496 324.632 791.321 371.389 776.67C473.225 744.761 454.388 674.793 380.49 658.503Z"
                fill="#F7D145"
                filter={`url(#${p("f4")})`}
              />
            </g>

            {/* Right arm */}
            <g transform={`rotate(${rightArmSway}, 553, 682)`}>
              <path
                d="M552.874 682.74C584.045 659.33 591.583 655.887 606.342 636.903C619.199 620.365 692.112 641.073 749.533 659.742C760.416 663.28 763.566 677.425 755.4 685.441C720.581 719.619 661.534 777.365 613.104 784.811C507.626 801.031 493.71 729.92 552.874 682.74Z"
                fill="#F7D145"
                filter={`url(#${p("f5")})`}
              />
            </g>

            {/* ── Book ────────────────────────────────────────────────────────── */}
            {/* Main book cover (dark olive) */}
            <path d="M366.834 605.758C368.683 602.057 376.987 592.011 381.254 592.383C408.251 594.737 436.477 602.812 459.911 616.63C465.265 619.787 469.559 624.796 474.061 628.96C474.701 629.543 475.54 630.129 476.208 629.548C481.342 625.807 484.853 621.407 490.394 617.941C510.736 605.231 534.082 598.944 557.707 596.032C562.544 595.435 567.954 595.148 572.979 594.655C577.394 599.671 580.15 602.346 583.239 608.326C588.298 608.121 590.903 607.741 593.196 612.879C594.369 618.039 590.505 653.904 589.323 660.255C592.138 658.219 586.49 662.566 589.323 660.255C585.206 672.626 579.377 704.61 571.212 719.446C562.902 721.361 554.329 722.45 546.104 724.38C537.223 726.465 528.311 728.728 519.394 730.692C512.89 732.123 504.754 736.021 498.409 738.373C495.228 739.527 494.432 741.626 493.557 741.964C488.162 751.018 465.519 751.041 458.75 742.964C457.888 741.939 457.005 740.71 456.116 739.646C447.167 736.623 438.164 732.321 428.938 729.798C419.077 727.106 408.865 724.865 399.015 721.971C394.508 720.646 383.936 718.151 379.995 715.868C378.598 709.686 360.052 653.859 360.052 653.859C358.389 643.431 358.43 632.659 356.754 622.161C356.219 618.814 355.775 610.703 357.861 608.132C361.393 605.419 362.447 605.541 366.834 605.758Z" fill="#808C46"/>
            {/* White pages */}
            <path d="M366.834 605.759C368.683 602.057 376.988 592.012 381.255 592.384C408.252 594.737 436.477 602.812 459.912 616.631C465.265 619.788 469.56 624.796 474.061 628.961C474.702 629.543 475.541 630.129 476.209 629.549C481.342 625.807 484.853 621.407 490.395 617.942C510.737 605.232 534.083 598.945 557.708 596.032C562.544 595.436 567.954 595.148 572.979 594.655C577.395 599.671 580.15 602.347 583.239 608.326C580.755 609.086 576.961 609.656 574.311 610.184L559.429 613.282C548.172 615.622 508.792 625.175 501.509 631.998C500.81 635.095 496.48 635.786 494.357 639.115L493.952 639.244C492.499 638.879 491.346 638.996 489.85 639.028C489.474 639.333 489.037 639.65 488.721 639.996L489.869 641.224C476.151 643.78 473.256 642.629 460.711 638.978C459.322 638.16 458.746 638.183 457.32 637.373L457.278 636.896C458.255 636.351 458.805 636.471 459.926 636.487L457.571 634.837C448.188 628.212 433.445 624.052 422.669 620.261C409.487 615.623 396.285 612.417 382.759 608.993C377.685 607.709 371.696 607.232 366.834 605.759Z" fill="#FCF7EF"/>
            {/* Page details */}
            <path d="M459.926 636.487C474.327 644.56 465.834 631.28 476.465 634.339C480.837 635.594 481.129 640.482 488.721 639.995L489.868 641.224C476.15 643.78 473.256 642.628 460.711 638.977C459.322 638.16 458.745 638.183 457.32 637.373L457.278 636.896C458.255 636.351 458.805 636.47 459.926 636.487Z" fill="#9F905A"/>
            <path d="M489.848 639.027C493.046 635.587 497.266 634.024 501.508 631.998C500.808 635.094 496.479 635.785 494.356 639.115L493.95 639.243C492.498 638.878 491.345 638.996 489.848 639.027Z" fill="#AAB25C"/>
            <path d="M366.832 605.758C371.694 607.232 377.683 607.708 382.757 608.992C396.284 612.416 409.486 615.623 422.668 620.26C433.444 624.051 448.186 628.212 457.569 634.837L459.925 636.487C458.804 636.47 458.254 636.351 457.276 636.896L457.319 637.372C458.744 638.182 459.32 638.16 460.71 638.977C459.227 638.775 457.299 638.298 456.016 638.869C455.513 637.956 452.336 637.869 450.628 637.336C444.727 634.839 437.696 631.654 431.744 629.495C420.309 625.448 408.711 621.881 396.979 618.803C392.556 617.673 387.532 616.654 383.049 615.594C375.673 613.853 365.165 609.953 357.677 611.049L357.313 611.105C357.544 610.076 357.693 609.173 357.859 608.131C361.391 605.419 362.446 605.541 366.832 605.758Z" fill="#BBCA7C"/>
            <path d="M457.568 634.837L459.924 636.487C458.803 636.47 458.253 636.35 457.276 636.895L457.318 637.372C458.743 638.182 459.319 638.159 460.709 638.977C459.226 638.775 457.298 638.298 456.015 638.869C455.512 637.956 452.335 637.869 450.627 637.336C453.709 637.544 454.987 636.753 457.568 634.837Z" fill="#BBCA7C"/>
            <path d="M583.239 608.325C588.298 608.121 590.903 607.74 593.196 612.878C590.879 613.516 589.424 612.147 586.57 612.812C557.966 619.472 528.144 624.314 501.553 637.346C499.413 638.394 496.361 637.968 495.104 640.147L493.951 639.243L494.357 639.114C496.48 635.784 500.809 635.094 501.508 631.997C508.791 625.174 548.171 615.62 559.428 613.281L574.31 610.183C576.96 609.655 580.755 609.085 583.239 608.325Z" fill="#BBCA7C"/>
            {/* Spine connector + decorative details */}
            <path d="M493.558 741.964L493.714 741.058C494.754 739.545 495.603 739.15 497.131 738.14L496.954 736.727L495.797 736.53C494.81 737.013 494.935 737.309 494.295 738.521L494.039 738.049L494.597 738.072L494.337 738.514L493.973 737.361L494.644 738.026L494.004 737.469C493.811 733.662 495.144 728.097 495.042 724.093C494.96 720.87 494.801 717.662 494.732 714.44C494.535 711.877 495.248 708.14 494.875 705.655C493.82 698.629 493.889 692.262 494.596 685.225C494.918 682.018 494.55 678.31 494.85 674.999C495.169 671.474 495.457 667.831 495.655 664.294C495.679 659.598 494.096 652.718 495.778 648.214C495.972 648.654 495.287 652.615 495.406 653.589C496.921 661.784 495.231 669.722 495.386 677.719C495.537 685.456 493.952 692.084 495.053 699.493C495.473 702.325 495.417 704.763 495.677 707.493C500.152 704.608 497.052 673.828 499.145 669.34L499.495 669.795C499.85 672.411 499.008 678.097 498.887 681.11L497.713 710.236C497.294 719.306 498.695 721.539 497.04 731.228C500.062 735.292 496.988 734.561 498.411 738.373C495.229 739.527 494.434 741.626 493.558 741.964Z" fill="#4F5722"/>
            <path d="M460.709 638.977C473.254 642.628 476.149 643.779 489.867 641.223C489.903 642.54 489.806 642.625 490.516 643.769C479.686 648.591 467.839 644.337 457.05 642.934C456.77 641.594 456.368 640.193 456.016 638.869C457.299 638.298 459.227 638.775 460.709 638.977Z" fill="#BBCA7C"/>
            <path d="M488.722 639.996C489.038 639.65 489.475 639.333 489.851 639.028C491.347 638.996 492.5 638.879 493.952 639.244L495.106 640.148C493.663 641.978 492.652 642.776 490.519 643.77C489.809 642.626 489.906 642.541 489.87 641.224L488.722 639.996Z" fill="#D7DC92"/>

          </g>
          {/* end book + arms sway group */}

          {/* ── Head group: drift + nod + squash ────────────────────────────── */}
          <g transform={`translate(${headDx}, ${headDy}) rotate(${headNod}, 493, 145)`}>
            <g transform={`translate(493, 145) scale(${headSquashX}, ${headSquashY}) translate(-493, -145)`}>

              {/* Neck shadows */}
              <g opacity={0.4} filter={`url(#${p("f2")})`}>
                <path d="M450.376 270.172C464.042 264.005 502.076 255.372 544.876 270.172C598.376 288.672 415.876 288.172 450.376 270.172Z" fill="#B23C05"/>
              </g>
              <g opacity={0.4} filter={`url(#${p("f3")})`}>
                <path d="M533.499 245.499C524.955 248.602 489.943 257.335 463.185 249.888C429.739 240.578 555.068 236.442 533.499 245.499Z" fill="#B23C05"/>
              </g>

              {/* Head circle */}
              <circle cx={493} cy={145} r={110} fill="#F7D145" filter={`url(#${p("f1")})`}/>

              {/* Left eye — blinks */}
              <g transform={`translate(411, 465) scale(1, ${eyeScaleY}) translate(-411, -465)`}>
                <path d="M411.479 428C419.678 428 423 432 424.408 434.321C431.455 442.807 434.448 450.812 435.286 461.939C436.53 478.451 428.58 501.025 409.175 501.922C402.907 502.212 396.782 499.978 392.176 495.714C372.967 478.168 379.456 428.811 411.479 428Z" fill="#1C170B"/>
                <g filter={`url(#${p("f6")})`}>
                  <path d="M402.588 435.31C405.111 435.115 406.117 435.015 408.224 436.218C409.447 437.699 409.293 438.305 409.365 440.116C410.178 440.625 410.896 441.111 411.693 441.647L411.902 442.956C419.012 456.194 406.032 468.295 397.002 457.028C387.107 457.791 393.025 445.603 396.043 441.344C398.036 438.531 399.867 437.302 402.588 435.31Z" fill="#FAF3EC"/>
                </g>
                <g filter={`url(#${p("f7")})`}>
                  <path d="M402.405 435.12C405.005 434.923 406.041 434.822 408.211 436.033C409.471 437.522 409.312 438.132 409.386 439.954C410.224 440.465 410.964 440.954 411.784 441.493L412 442.811C408.557 441.118 406.625 439.187 402.54 440.654C395.773 443.086 394.268 451.112 396.652 456.966C386.459 457.733 392.555 445.473 395.664 441.189C397.717 438.36 399.602 437.123 402.405 435.12Z" fill="#3A372F"/>
                </g>
              </g>

              {/* Right eye — blinks */}
              <g transform={`translate(589, 466) scale(1, ${eyeScaleY}) translate(-589, -466)`}>
                <path d="M589.369 428.706C621.867 428.523 630.994 493.598 594.351 502.663C555.685 504.419 554.456 433.119 589.369 428.706Z" fill="#1C170B"/>
                <g filter={`url(#${p("f8")})`}>
                  <path d="M576.49 452.759C577.096 454.049 577.139 454.759 576.608 455.979C569.333 454.164 573.451 439.586 580.006 437.664C584.199 436.436 587.823 438.013 589.305 442.115C592.618 444.137 594.846 446.01 595.748 450.049C596.354 452.791 595.844 455.661 594.33 458.027C589.037 466.354 580.302 462.46 578.514 452.619C577.655 451.775 577.929 451.624 577.757 450.079L577.59 450.499L577.886 450.8L577.386 452.615L576.49 452.759Z" fill="#FAF3EC"/>
                </g>
                <g filter={`url(#${p("f9")})`}>
                  <path d="M576.06 452.732C576.72 454.041 576.766 454.762 576.188 456C568.275 454.158 572.754 439.363 579.885 437.413C584.446 436.166 588.388 437.766 590 441.93L585.246 442.04C580.159 445.421 579.418 446.592 578.261 452.59C577.327 451.734 577.625 451.58 577.438 450.013L577.257 450.438L577.578 450.743L577.035 452.586L576.06 452.732Z" fill="#312E24"/>
                </g>
                <g filter={`url(#${p("f10")})`}>
                  <path d="M576.49 452.759L575.947 452.886L575.475 452.235C575.11 444.84 575.121 438.674 584.935 442.224C580.259 445.556 579.577 446.709 578.514 452.619C577.655 451.776 577.928 451.624 577.757 450.08L577.59 450.499L577.886 450.8L577.386 452.615L576.49 452.759Z" fill="#534639"/>
                </g>
              </g>

              {/* Left cheek */}
              <path d="M354.002 488.785C366.292 488.07 381.734 490.477 385.001 505.019C386.026 509.579 385.143 514.363 382.556 518.257C378.409 524.432 372.217 526.795 365.337 528.245C353.923 529.158 338.873 527.064 334.774 514.24C333.375 509.718 333.887 504.821 336.192 500.686C339.888 493.968 346.962 490.735 354.002 488.785Z" fill="#F9A6A0"/>
              <g filter={`url(#${p("f11")})`}>
                <path d="M368 494C373.243 494.048 380.362 498.673 380 504C375.832 504.091 367.526 498.087 368 494Z" fill="#FDC3BF"/>
              </g>

              {/* Right cheek */}
              <path d="M626.148 494.285C641.879 485.407 671.15 495.187 664.863 516.522C657.953 539.968 605.956 533.98 615.078 505.471C615.733 503.36 618.573 499.408 620.253 497.867C621.59 496.68 624.468 495.224 626.148 494.285Z" fill="#EF928B"/>
              <g filter={`url(#${p("f12")})`}>
                <path d="M632.016 497C626.773 497.048 619.653 501.673 620.016 507C624.184 507.091 632.49 501.087 632.016 497Z" fill="#FDC3BF"/>
              </g>

              {/* Focused mouth */}
              <path d="M471.504 494.784C471.5 491.499 475 489 478.415 490.134C480.5 491.5 480.949 493.63 482.46 495.842C489.37 505.97 498.06 507.141 509.126 502.936C514.766 498.973 514.929 497.593 518.612 491.664C528.418 484.735 532.463 504.579 511.184 513.085C503.114 516.238 494.123 516.055 486.186 512.586C478.626 509.187 473.047 503.065 471.504 494.784Z" fill="#1C170B"/>
              <path d="M509.127 502.936C514.767 498.973 514.929 497.593 518.612 491.664L520.234 492.572C521.198 496.986 512.309 506.706 507.958 505.884L507.711 505.234L509.127 502.936Z" fill="#312E24"/>

            </g>
          </g>
          {/* end head group */}

        </g>
        {/* end bob group */}

      </svg>
    </AbsoluteFill>
  );
};
