import React from "react";
import {
  AbsoluteFill,
  interpolate,
  useCurrentFrame,
  useVideoConfig,
} from "remotion";

/**
 * NewMascotHatWithBag — hatwithbag.svg idle animation.
 *
 * Idle pattern:
 * • Smooth body bob (1.2 Hz)
 * • Head drift + squash/stretch
 * • Hat tracks head drift (inside head group)
 * • Bag has gentle pendulum sway (slight phase lag)
 * • Left + right arms sway in opposite phases
 * • Both eyes blink every ~3.5s
 */
export const NewMascotHatWithBag: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps, width, height } = useVideoConfig();
  const p = (k: string) => `nmhb-${k}`;

  // ── Body bob — idle 1.2 Hz ───────────────────────────────────────────────
  const bob = Math.sin((frame / fps) * Math.PI * 1.2) * 10;

  // ── Head drift + squash ─────────────────────────────────────────────────
  const dotPhase = (frame / fps) * Math.PI;
  const headDx = Math.sin(dotPhase * 0.7) * 5;
  const headDy = Math.sin(dotPhase) * 7;
  const press = Math.max(0, Math.sin(dotPhase));
  const headSquashY = 1 - 0.08 * press;
  const headSquashX = 1 + 0.05 * press;

  // ── Left arm — gentle idle sway ──────────────────────────────────────────
  const leftArmAngle = Math.sin((frame / fps) * Math.PI * 0.8) * 8;

  // ── Right arm — opposite phase ───────────────────────────────────────────
  const rightArmAngle = Math.sin((frame / fps) * Math.PI * 0.8 + Math.PI) * 8;

  // ── Bag pendulum — hangs naturally, slight phase lag behind body ──────────
  const bagSwing = Math.sin((frame / fps) * Math.PI * 1.0 + 0.45) * 3;

  // ── Blink — both eyes every ~3.5s ───────────────────────────────────────
  const blinkPeriod = Math.round(fps * 3.5);
  const blinkDur = Math.round(fps * 0.13);
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
          <filter id={p("f0")} x="90.3856" y="238.634" width="765.268" height="762.131" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
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
          <filter id={p("f4")} x="138.458" y="555.812" width="155.093" height="272.386" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
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

          {/* Left eye highlight */}
          <filter id={p("f5")} x="390.218" y="433.891" width="25.0343" height="28.893" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.65"/>
          </filter>
          <filter id={p("f6")} x="390.3" y="434.3" width="22.4" height="23.4" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.35"/>
          </filter>

          {/* Right eye highlight */}
          <filter id={p("f7")} x="570.859" y="435.358" width="27.0394" height="29.1125" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.95"/>
          </filter>
          <filter id={p("f8")} x="571.3" y="436.3" width="19.4" height="20.4" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.35"/>
          </filter>
          <filter id={p("f9")} x="574.668" y="440.492" width="10.9676" height="13.0943" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.35"/>
          </filter>

          {/* Cheek highlights */}
          <filter id={p("f10")} x="366.181" y="492.2" width="15.6323" height="13.601" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.9"/>
          </filter>
          <filter id={p("f11")} x="618.2" y="495.2" width="15.6323" height="13.601" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.9"/>
          </filter>

          {/* Right arm */}
          <filter id={p("f12")} x="645" y="555.9" width="155.093" height="272.386" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
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

          {/* Hat shadow */}
          <filter id={p("f13")} x="567.445" y="201.763" width="233.831" height="206.228" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="1.4"/>
          </filter>

          {/* Hat brim buckle details */}
          <filter id={p("f14")} x="713.479" y="233.663" width="31.9914" height="30.3459" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.2"/>
          </filter>
          <filter id={p("f15")} x="719.175" y="229.175" width="24.1947" height="21.3059" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.2"/>
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
            d="M270.548 382.714C175.869 479.647 86.1401 654.573 127.915 829.517C145.272 881.371 165.202 911.976 222.935 941.975C253.337 957.772 327.5 950.5 375.544 921.664L445.394 890.456C490.742 873.851 509.572 876.412 538.5 889.192C577.029 910.413 587.5 931.5 649.207 964.222C729.487 1006.79 793.126 956.041 817.513 889.192C874.808 742.915 814.513 422.978 650.331 310.479C516.054 226.594 403.003 247.226 270.548 382.714Z"
            fill="#F7D145"
            filter={`url(#${p("f0")})`}
          />

          {/* ── Left arm — gentle idle sway ─────────────────────────────────── */}
          <g transform={`rotate(${leftArmAngle}, 226, 578)`}>
            <path
              d="M257.7 773.068C271.728 736.698 272.987 728.506 287.23 709.133C299.638 692.255 259.842 627.746 226.232 577.586C219.862 568.08 205.393 568.903 199.906 578.945C176.511 621.76 137.044 694.31 143.077 742.936C156.218 848.842 228.429 842.851 257.7 773.068Z"
              fill="#F7D145"
              filter={`url(#${p("f4")})`}
            />
          </g>

          {/* ── Bag — gentle pendulum sway (rotate around upper strap point) ── */}
          <g transform={`rotate(${bagSwing}, 809, 562)`}>
            {/* Bag strap (line 53) */}
            <path d="M809.219 562.116C810.943 564.299 813.643 569.352 814.5 571.999C813.437 573.914 664.398 718.453 662.943 720.135C660.931 722.463 656.504 724.217 655.73 726.823C655.652 725.678 651.798 713.917 651.256 711.76C651.947 712.338 808.372 561.734 809.219 562.116Z" fill="#7F573A"/>

            {/* Main bag body + cross-body strap */}
            <path d="M190.806 491C194.519 504.309 205.784 521.535 214.752 532.161C267.81 595.038 336.55 644.105 407.584 684.151C424.994 693.869 442.737 702.972 460.779 711.456C466.01 713.902 471.508 716.101 476.821 718.438C486.209 722.572 497.576 729.565 507.585 731.015C508.887 730.566 509.441 730.354 510.051 729.059C510.924 727.206 511.087 724.992 511.928 723.109C513.661 719.228 519.02 715.858 522.76 714.171C545.038 704.118 619.341 691.939 641.08 700.392C646.086 702.337 648.196 705.609 650.735 710.016L651.256 711.761C651.798 713.918 655.652 725.679 655.73 726.824C656.313 732.3 659.858 738.575 660.848 744.412C665.317 770.752 670.972 801.777 653.299 824.737C641.266 840.363 615.91 848.362 596.854 851.092C567.669 855.272 534.037 845.281 522.487 815.613C514.088 801.849 511.086 767.005 510.241 750.61C510.01 749.877 509.856 748.891 509.096 748.514C497.021 742.507 481.675 738.616 470.056 732.016C466.407 730.87 458.211 726.897 454.524 725.132C443.472 719.899 432.571 714.356 421.835 708.504C410.521 702.477 398.821 696.635 387.712 690.298C345.935 666.657 306.427 639.208 269.69 608.311C252.994 594.346 239.89 583.267 224.877 566.974C212.893 553.877 201.927 539.881 192.075 525.112C189.416 521.122 185.689 512.36 183.837 510.089L183 510.239C183.378 502.266 183.782 496.434 190.806 491Z" fill="#C89F7B"/>
            {/* Bag shadow */}
            <path d="M650.735 710.017L651.256 711.761C651.798 713.918 655.652 725.679 655.73 726.825C656.313 732.3 659.858 738.575 660.848 744.412C665.317 770.752 670.972 801.777 653.299 824.737C641.266 840.363 615.91 848.362 596.854 851.092C567.669 855.272 534.037 845.281 522.488 815.613C523.675 816.49 525.014 818.838 525.438 818.926C529.443 819.742 535.002 816.01 538.423 814.421C540.961 816.568 544.646 826.455 547.886 828.829C552.582 832.271 563.242 824.329 565.915 820.33C568.01 817.192 570.559 816.129 573.603 814.214C577.881 811.521 577.133 808.853 575.642 804.673C572.442 794.604 562.886 796.219 555.518 792.044C552.318 790.233 547.803 784.386 544.873 781.842C541.384 778.519 540.358 770.607 537.901 766.856C535.877 763.764 535.018 762.257 533.54 758.79L533.794 758.366C536.038 761.865 538.417 766.164 540.982 769.25L541.39 769.73C542.045 771.531 545.048 774.158 546.575 775.67C557.984 784.845 571.555 786.987 585.327 790.471L589.455 791.033C588.258 787.895 587.448 786.466 588.304 783.277C590.482 782.864 590.807 783.179 592.185 781.935C594.357 772.93 596.952 772.331 605.456 770.855C608.005 772.202 613.201 775.851 615.807 777.569C623.526 776.908 638.253 766.52 644.213 761.489C655.678 751.642 654.992 735.747 652.758 722.17C651.984 717.463 650.389 715.022 650.735 710.017Z" fill="#A78160"/>
            {/* Bag clasp/hardware */}
            <path d="M588.304 783.276C590.482 782.863 590.807 783.178 592.184 781.934C594.357 772.929 596.952 772.33 605.456 770.854C608.005 772.201 613.201 775.85 615.807 777.568C617.928 780.536 618.975 782.615 618.227 786.434C616.643 794.542 606.854 800.378 598.944 798.546C593.954 797.396 592.138 794.831 589.455 791.032C588.258 787.895 587.448 786.465 588.304 783.276Z" fill="#252525"/>
            <path d="M546.575 775.671C557.984 784.846 571.555 786.988 585.327 790.472C579.651 790.905 574.29 790.487 568.66 789.765C565.378 789.341 561.968 787.773 559.016 789.429C555.75 788.314 548.196 779.02 546.575 775.671Z" fill="#A2795A"/>
            <path d="M541.39 769.728C542.742 770.972 544.986 773.433 546.462 773.929C545.956 771.519 543.784 769.269 542.06 767.571L542.019 766.761C543.526 767.839 545.9 771.034 548.314 772.866C556.219 778.878 565.033 781.18 574.739 782.145C578.082 782.475 585.9 782.614 588.304 783.275C587.448 786.464 588.258 787.893 589.455 791.031L585.327 790.469C571.555 786.985 557.984 784.844 546.575 775.668C545.048 774.156 542.045 771.529 541.39 769.728Z" fill="#7F573A"/>
            <path d="M523.367 725.967C527.722 733.367 523.136 746.774 517.744 752.941C516.114 752.239 511.561 749.726 510.241 750.609C510.01 749.876 509.856 748.89 509.096 748.513C497.021 742.507 481.675 738.615 470.056 732.015C478.256 731.891 508.597 751.61 517.329 745.835C518.575 745.009 518.464 740.107 518.519 738.584C518.562 737.382 516.716 735.38 515.804 734.291C518.237 735.622 520.09 737.939 522.881 737.527C524.577 735.38 523.149 729.378 523.367 725.967Z" fill="#A2795A"/>
            <path d="M511.692 732.274C514.378 727.898 517.7 720.111 523.367 725.968C523.149 729.379 524.577 735.381 522.881 737.528C520.09 737.941 518.237 735.624 515.804 734.292L511.692 732.274Z" fill="#7F573A"/>
            {/* Bag buckle dots */}
            <path d="M577.638 756.983C587.05 758.165 589.682 765.375 580.714 770.473C574.878 770.525 566.988 760.818 577.638 756.983Z" fill="#090909"/>
            <path d="M616.684 750.234C624.507 752.18 628.052 760.168 619.594 763.657C612.2 762.413 609.455 754.595 616.684 750.234Z" fill="#090909"/>
          </g>
          {/* end bag sway group */}

          {/* ── Right arm — opposite phase idle sway ─────────────────────────── */}
          <g transform={`rotate(${rightArmAngle}, 712, 577)`}>
            <path
              d="M680.851 773.156C666.823 736.786 665.565 728.594 651.321 709.221C638.913 692.343 678.709 627.834 712.32 577.674C718.689 568.167 733.158 568.991 738.645 579.033C762.04 621.848 801.508 694.398 795.474 743.024C782.333 848.93 710.122 842.939 680.851 773.156Z"
              fill="#F7D145"
              filter={`url(#${p("f12")})`}
            />
          </g>

          {/* ── Head group: drift + squash ───────────────────────────────────── */}
          {/* Hat sits outside squash group so it translates with head but isn't squashed */}
          <g transform={`translate(${headDx}, ${headDy})`}>

            {/* ── Hat cluster — moves with head drift ──────────────────────────── */}
            {/* Main hat shape */}
            <path d="M592.564 204.562C607.021 195.397 635.862 196.42 652.061 200.586C669.129 204.976 688.579 216.966 702.096 228.128C703.73 229.477 708.969 233.276 709.97 234.778C711.752 236.207 713.143 237.274 714.765 238.914C720.623 232.413 730.35 221.154 739.527 229.576C741.362 231.394 741.841 231.772 742.97 234.065C750.161 247.715 736.817 249.238 733.951 257.948C737.268 259.252 738.218 259.709 741.137 261.794C746.103 265.633 751.311 268.925 756.191 272.574C763.67 278.166 773.466 288.52 779.17 296.14C800.143 326.203 810.361 370.412 776.386 395.979C770.172 400.655 761.949 403.063 754.398 404.274C752.549 404.568 748.18 404.866 746.894 405.19C743.4 401.593 732.333 397.104 727.379 394.143C720.883 390.263 715.221 386.898 709.097 382.265C669.009 351.945 633.45 315.951 590.711 289.015C587.406 286.932 580.004 281.64 576.606 280.833C576.388 278.189 573.208 274.172 572.492 271.566C571.48 267.884 572.132 263.761 570.563 260.173C570 258 567 241.5 579.833 213.534C583.673 209.342 587.47 206.909 592.564 204.562Z" fill="#DFB690"/>
            {/* Hat shadow */}
            <g filter={`url(#${p("f13")})`}>
              <path d="M592.564 204.562C591.447 206.783 587.524 208.421 585.13 209.576C586.254 212.787 587.824 215.965 588.781 218.827C590.975 225.914 592.128 232.951 593.829 240.197C594.893 244.718 593.07 255.565 595.151 259.258C597.787 263.922 603.83 268.245 608.131 271.297C614.606 275.888 620.783 281.475 627.421 285.8C632.008 288.79 634.424 297.602 639.399 299.206C649.624 302.504 654.929 307.446 663.016 314.071C664.967 315.669 671.953 315.939 673.816 317.558C673.698 320.411 672.629 321.171 673.294 322.927L673.98 323L673.853 321.37C674.461 321.232 680.778 324.934 681.518 325.554C688.109 331.054 690.318 328.802 696.474 330.1C707.059 332.333 722.113 341.057 729.25 327.143C732.065 321.34 732.972 313.831 733.802 307.401C738.505 303.44 738.256 298.237 743.76 298.087C745.841 298.03 748.248 300.127 749.389 301.686C754.916 309.32 753.163 308.655 761.793 311.088C770.184 313.453 769.519 312.624 771.375 303.286C772.156 299.334 774.41 297.646 778.273 297L779.17 296.14C800.143 326.202 810.361 370.412 776.386 395.979C770.172 400.655 761.949 403.063 754.398 404.274C752.549 404.568 748.18 404.866 746.894 405.19C743.4 401.593 732.333 397.103 727.379 394.143C720.883 390.263 715.221 386.898 709.097 382.265C669.009 351.945 633.45 315.951 590.711 289.014C587.406 286.932 580.004 281.64 576.606 280.833C576.388 278.189 573.208 274.172 572.492 271.565C571.48 267.884 572.132 263.761 570.563 260.173C571.5 257 566 241 579.833 213.534C583.673 209.341 587.47 206.909 592.564 204.562Z" fill="#B38C69"/>
            </g>
            {/* Hat brim band + buckle */}
            <path d="M714.765 238.914C720.623 232.413 730.35 221.154 739.527 229.576C741.362 231.394 741.841 231.772 742.969 234.065C750.161 247.715 736.816 249.238 733.95 257.948C733.119 259.752 732.692 260.091 730.788 261.027C728.436 261.839 726.439 262.467 724.205 263.61C721.928 262.444 720.647 261.881 718.612 260.278L718.538 259.639C719.194 258.757 719.251 258.672 719.709 257.672L719.303 257.339C716.774 255.217 715.494 254.49 713.879 251.509C709.303 245.13 713.844 244.822 714.765 238.914Z" fill="#E3B88E"/>
            <g filter={`url(#${p("f14")})`}>
              <path d="M742.97 234.064C750.161 247.714 736.817 249.237 733.951 257.947C733.119 259.751 732.692 260.09 730.788 261.027C728.436 261.838 726.439 262.466 724.205 263.609C721.928 262.443 720.647 261.88 718.612 260.277L718.538 259.639C719.194 258.756 719.251 258.671 719.709 257.671L719.303 257.338C716.774 255.216 715.494 254.489 713.879 251.508C715.186 252.526 715.937 253.15 717.382 253.952C719.402 253.771 722.211 251.19 725.08 250.081C726.718 250.078 728.004 248.384 729.274 247.161C731.693 244.658 735.553 244.158 738.485 242.399C741.486 240.594 741.47 237.278 742.8 234.423L742.97 234.064Z" fill="#A77754"/>
            </g>
            <path d="M718.612 260.277C720.536 259.611 727.675 258.508 730.286 257.905L730.896 258.523L730.788 261.027C728.436 261.839 726.439 262.467 724.205 263.609C721.928 262.443 720.647 261.88 718.612 260.277Z" fill="#7A5131"/>
            <g filter={`url(#${p("f15")})`}>
              <path d="M725.08 250.081L725.084 247.877C723.541 248.214 720.78 248.396 719.575 247.335C719.96 247.338 723.431 247.301 723.875 247.005C727.858 244.38 735.609 240.411 738.509 237.023C739.363 236.026 738.319 233.461 737.952 232.2C738.39 230.254 737.964 231.146 739.527 229.576C741.362 231.393 741.841 231.771 742.97 234.064L742.8 234.423C741.47 237.278 741.486 240.595 738.485 242.399C735.553 244.158 731.693 244.658 729.274 247.161C728.004 248.384 726.718 250.078 725.08 250.081Z" fill="#BC8860"/>
            </g>
            <path d="M724.165 264.822C719.183 262.104 713.494 261.67 710.94 259.49C710.188 256.533 711.245 255.826 709.401 252.987C707.998 250.835 705.817 248.693 707.119 246.242C708.004 246.638 708.529 248.022 709.156 249.08L709.811 249.135C711.329 246.311 712.472 243.139 713.641 240.136C713.359 238.355 711.459 237.443 709.695 235.482L709.97 234.777C711.752 236.206 713.143 237.273 714.765 238.913C713.844 244.821 709.303 245.129 713.879 251.509C715.494 254.49 716.774 255.217 719.303 257.339L719.709 257.672C719.251 258.672 719.194 258.757 718.538 259.639L718.612 260.277C720.647 261.88 721.928 262.443 724.205 263.609C726.439 262.467 728.436 261.839 730.788 261.027C732.692 260.09 733.119 259.751 733.95 257.947C737.268 259.251 738.218 259.709 741.136 261.794C736.251 261.368 728.57 262.579 724.165 264.822Z" fill="#BC8860"/>

            {/* ── Head content: squash/stretch ────────────────────────────────── */}
            <g transform={`translate(493, 145) scale(${headSquashX}, ${headSquashY}) translate(-493, -145)`}>

              {/* Neck shadows */}
              <g opacity={0.4} filter={`url(#${p("f2")})`}>
                <path d="M450.376 270.172C464.042 264.005 502.076 255.372 544.876 270.172C598.376 288.672 415.876 288.172 450.376 270.172Z" fill="#B23C05"/>
              </g>
              <g opacity={0.4} filter={`url(#${p("f3")})`}>
                <path d="M533.5 245.499C524.956 248.602 489.943 257.335 463.186 249.888C429.739 240.578 555.068 236.442 533.5 245.499Z" fill="#B23C05"/>
              </g>

              {/* Head circle */}
              <circle cx={493} cy={145} r={110} fill="#F7D145" filter={`url(#${p("f1")})`}/>

              {/* Left eye — blinks */}
              <g transform={`translate(411, 465) scale(1, ${eyeScaleY}) translate(-411, -465)`}>
                <path d="M411.48 428C419.679 428 423 432 424.408 434.321C431.456 442.807 434.448 450.812 435.286 461.939C436.531 478.451 428.581 501.025 409.176 501.922C402.907 502.212 396.783 499.978 392.177 495.714C372.967 478.168 379.456 428.811 411.48 428Z" fill="#1C170B"/>
                <g filter={`url(#${p("f5")})`}>
                  <path d="M402.589 435.31C405.113 435.115 406.119 435.015 408.226 436.218C409.449 437.699 409.295 438.305 409.367 440.116C410.18 440.625 410.898 441.111 411.694 441.647L411.904 442.956C419.014 456.194 406.034 468.295 397.004 457.028C387.109 457.791 393.027 445.603 396.045 441.344C398.038 438.531 399.869 437.302 402.589 435.31Z" fill="#FAF3EC"/>
                </g>
                <g filter={`url(#${p("f6")})`}>
                  <path d="M402.405 435.12C405.005 434.923 406.041 434.822 408.211 436.033C409.471 437.522 409.312 438.132 409.386 439.954C410.224 440.465 410.964 440.954 411.784 441.493L412 442.811C408.557 441.118 406.625 439.187 402.54 440.654C395.773 443.086 394.268 451.112 396.652 456.966C386.459 457.733 392.555 445.473 395.664 441.189C397.717 438.36 399.602 437.123 402.405 435.12Z" fill="#3A372F"/>
                </g>
              </g>

              {/* Right eye — blinks */}
              <g transform={`translate(589, 466) scale(1, ${eyeScaleY}) translate(-589, -466)`}>
                <path d="M589.37 428.706C621.867 428.523 630.994 493.598 594.352 502.663C555.686 504.419 554.456 433.119 589.37 428.706Z" fill="#1C170B"/>
                <g filter={`url(#${p("f7")})`}>
                  <path d="M576.491 452.759C577.097 454.049 577.14 454.759 576.609 455.979C569.334 454.164 573.452 439.586 580.007 437.664C584.2 436.436 587.824 438.013 589.306 442.115C592.619 444.137 594.847 446.01 595.749 450.049C596.355 452.791 595.845 455.661 594.331 458.027C589.038 466.354 580.303 462.46 578.515 452.619C577.656 451.775 577.93 451.624 577.758 450.079L577.591 450.499L577.887 450.8L577.387 452.615L576.491 452.759Z" fill="#FAF3EC"/>
                </g>
                <g filter={`url(#${p("f8")})`}>
                  <path d="M576.06 452.732C576.72 454.041 576.766 454.762 576.188 456C568.275 454.158 572.754 439.363 579.885 437.413C584.446 436.166 588.388 437.766 590 441.93L585.246 442.04C580.159 445.421 579.418 446.592 578.261 452.59C577.327 451.734 577.625 451.58 577.438 450.013L577.257 450.438L577.578 450.743L577.035 452.586L576.06 452.732Z" fill="#312E24"/>
                </g>
                <g filter={`url(#${p("f9")})`}>
                  <path d="M576.49 452.759L575.948 452.886L575.475 452.235C575.11 444.84 575.121 438.674 584.935 442.224C580.259 445.556 579.577 446.709 578.514 452.619C577.655 451.776 577.929 451.624 577.757 450.08L577.591 450.499L577.886 450.8L577.387 452.615L576.49 452.759Z" fill="#534639"/>
                </g>
              </g>

              {/* Left cheek */}
              <path d="M354.002 488.785C366.292 488.07 381.734 490.477 385.001 505.019C386.026 509.579 385.143 514.363 382.556 518.257C378.409 524.432 372.217 526.795 365.337 528.245C353.923 529.158 338.873 527.064 334.774 514.24C333.375 509.718 333.887 504.821 336.192 500.686C339.888 493.968 346.962 490.735 354.002 488.785Z" fill="#F9A6A0"/>
              <g filter={`url(#${p("f10")})`}>
                <path d="M368 494C373.244 494.048 380.363 498.673 380 504C375.832 504.091 367.526 498.087 368 494Z" fill="#FDC3BF"/>
              </g>

              {/* Right cheek */}
              <path d="M626.146 494.285C641.877 485.407 671.147 495.187 664.86 516.522C657.951 539.968 605.954 533.98 615.075 505.471C615.73 503.36 618.571 499.408 620.251 497.867C621.588 496.68 624.466 495.224 626.146 494.285Z" fill="#EF928B"/>
              <g filter={`url(#${p("f11")})`}>
                <path d="M632.013 497C626.77 497.048 619.65 501.673 620.013 507C624.181 507.091 632.487 501.087 632.013 497Z" fill="#FDC3BF"/>
              </g>

              {/* Smirk mouth */}
              <path d="M526.825 509.24C529.058 533.416 509.441 544.063 495.563 543.494C475.913 542.688 463.184 521.332 466.534 509.243C469.883 501.177 484.398 506.216 493.33 507.228C497.024 507.228 500.679 506.536 504.267 505.661C512.377 503.684 525.077 502.133 526.825 509.24Z" fill="#03050D"/>
              <path d="M515.456 529.644C505.491 517.086 486.755 521.664 479 530.01C477.284 531.857 477.679 534.691 479.632 536.284C489.72 544.518 503.637 544.538 514.5 536.344C516.626 534.74 517.111 531.73 515.456 529.644Z" fill="#E06B51"/>

            </g>
            {/* end squash group */}
          </g>
          {/* end head drift group */}

        </g>
        {/* end bob group */}

      </svg>
    </AbsoluteFill>
  );
};
