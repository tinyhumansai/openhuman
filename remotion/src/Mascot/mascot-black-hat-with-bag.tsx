import React from "react";
import {
  AbsoluteFill,
  interpolate,
  useCurrentFrame,
  useVideoConfig,
} from "remotion";

export const BlackMascotHatWithBag: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps, width, height } = useVideoConfig();
  const p = (k: string) => `bmhb-${k}`;

  // ── Body bob ─────────────────────────────────────────────────────────────
  const bob = Math.sin((frame / fps) * Math.PI * 1.2) * 10;

  // ── Head drift + squash ─────────────────────────────────────────────────
  const dotPhase = (frame / fps) * Math.PI;
  const headDx = Math.sin(dotPhase * 0.7) * 5;
  const headDy = Math.sin(dotPhase) * 7;
  const press = Math.max(0, Math.sin(dotPhase));
  const headSquashY = 1 - 0.08 * press;
  const headSquashX = 1 + 0.05 * press;

  // ── Left arm — gentle idle sway ─────────────────────────────────────────
  const leftArmAngle = Math.sin((frame / fps) * Math.PI * 0.8) * 8;

  // ── Right arm — opposite phase ──────────────────────────────────────────
  const rightArmAngle = Math.sin((frame / fps) * Math.PI * 0.8 + Math.PI) * 8;

  // ── Bag pendulum ────────────────────────────────────────────────────────
  const bagSwing = Math.sin((frame / fps) * Math.PI * 1.0 + 0.45) * 3;

  // ── Blink ───────────────────────────────────────────────────────────────
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

          {/* Body — from blackhatwithbag.svg filter0 */}
          <filter id={p("f0")} x="90.3848" y="238.634" width="765.268" height="762.131" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="17" dy="28"/><feGaussianBlur stdDeviation="10.45"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.439078 0 0 0 0 0.439078 0 0 0 0 0.439078 0 0 0 1 0"/>
            <feBlend mode="normal" in2="shape" result="e1"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="-27" dy="-22"/><feGaussianBlur stdDeviation="29.75"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.0229492 0 0 0 0 0.0207891 0 0 0 0 0.0161271 0 0 0 1 0"/>
            <feBlend mode="normal" in2="e1" result="e2"/>
            <feTurbulence type="fractalNoise" baseFrequency="0.999" numOctaves={3} seed={8703}/>
            <feDisplacementMap in="e2" scale={8} xChannelSelector="R" yChannelSelector="G" width="100%" height="100%"/>
          </filter>

          {/* Head circle — from blackhatwithbag.svg filter1 */}
          <filter id={p("f1")} x="379" y="22" width="233" height="237" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="9" dy="2"/><feGaussianBlur stdDeviation="5.65"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.439216 0 0 0 0 0.439216 0 0 0 0 0.439216 0 0 0 1 0"/>
            <feBlend mode="normal" in2="shape" result="e1"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="-2" dy="-13"/><feGaussianBlur stdDeviation="19.7"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.0235294 0 0 0 0 0.0196078 0 0 0 0 0.0156863 0 0 0 1 0"/>
            <feBlend mode="normal" in2="e1" result="e2"/>
            <feTurbulence type="fractalNoise" baseFrequency="0.999" numOctaves={3} seed={8703}/>
            <feDisplacementMap in="e2" scale={8} xChannelSelector="R" yChannelSelector="G" width="100%" height="100%"/>
          </filter>

          {/* Neck shadows — filter2, filter3 */}
          <filter id={p("f2")} x="423.5" y="239.5" width="153.771" height="66.8599" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="11.25"/>
          </filter>
          <filter id={p("f3")} x="434.975" y="217.947" width="123.537" height="57.3708" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="11.25"/>
          </filter>

          {/* Left arm — from blackhatwithbag.svg filter4 */}
          <filter id={p("f4")} x="138.457" y="555.812" width="155.094" height="272.386" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="1" dy="-20"/><feGaussianBlur stdDeviation="7.55"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.439216 0 0 0 0 0.439216 0 0 0 0 0.439216 0 0 0 1 0"/>
            <feBlend mode="normal" in2="shape" result="e1"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="3" dy="-8"/><feGaussianBlur stdDeviation="3.55"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.0235294 0 0 0 0 0.0196078 0 0 0 0 0.0156863 0 0 0 0.8 0"/>
            <feBlend mode="normal" in2="e1" result="e2"/>
            <feTurbulence type="fractalNoise" baseFrequency="0.999" numOctaves={3} seed={8703}/>
            <feDisplacementMap in="e2" scale={8} xChannelSelector="R" yChannelSelector="G" width="100%" height="100%"/>
          </filter>

          {/* Left eye highlights — filter5, filter6 */}
          <filter id={p("f5")} x="390.218" y="433.891" width="25.0336" height="28.893" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.65"/>
          </filter>
          <filter id={p("f6")} x="390.3" y="434.3" width="22.4" height="23.4" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.35"/>
          </filter>

          {/* Right eye highlights — filter7, filter8, filter9 */}
          <filter id={p("f7")} x="570.858" y="435.358" width="27.0402" height="29.112" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.95"/>
          </filter>
          <filter id={p("f8")} x="571.3" y="436.3" width="19.4" height="20.4" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.35"/>
          </filter>
          <filter id={p("f9")} x="574.667" y="440.492" width="10.9684" height="13.0938" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.35"/>
          </filter>

          {/* Cheek highlights — filter10, filter11 */}
          <filter id={p("f10")} x="366.18" y="492.2" width="15.6332" height="13.601" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.9"/>
          </filter>
          <filter id={p("f11")} x="618.198" y="495.2" width="15.6332" height="13.601" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.9"/>
          </filter>

          {/* Right arm — from blackhatwithbag.svg filter12 */}
          <filter id={p("f12")} x="644.998" y="555.9" width="155.094" height="272.386" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dx="1" dy="-20"/><feGaussianBlur stdDeviation="7.55"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.439216 0 0 0 0 0.439216 0 0 0 0 0.439216 0 0 0 1 0"/>
            <feBlend mode="normal" in2="shape" result="e1"/>
            <feColorMatrix in="SourceAlpha" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 127 0" result="hardAlpha"/>
            <feOffset dy="-8"/><feGaussianBlur stdDeviation="3.55"/>
            <feComposite in2="hardAlpha" operator="arithmetic" k2="-1" k3="1"/>
            <feColorMatrix type="matrix" values="0 0 0 0 0.0235294 0 0 0 0 0.0196078 0 0 0 0 0.0156863 0 0 0 0.8 0"/>
            <feBlend mode="normal" in2="e1" result="e2"/>
            <feTurbulence type="fractalNoise" baseFrequency="0.999" numOctaves={3} seed={8703}/>
            <feDisplacementMap in="e2" scale={8} xChannelSelector="R" yChannelSelector="G" width="100%" height="100%"/>
          </filter>

          {/* Hat shadow — filter13 */}
          <filter id={p("f13")} x="567.446" y="201.762" width="233.83" height="206.228" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="1.4"/>
          </filter>

          {/* Hat buckle details — filter14, filter15 */}
          <filter id={p("f14")} x="713.479" y="233.665" width="31.9914" height="30.3453" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.2"/>
          </filter>
          <filter id={p("f15")} x="719.174" y="229.176" width="24.1945" height="21.3057" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
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

        {/* Everything bobs */}
        <g transform={`translate(0, ${bob})`}>

          {/* Body */}
          <path
            d="M270.547 382.715C175.868 479.648 86.1392 654.573 127.914 829.517C145.271 881.371 165.201 911.977 222.934 941.975C253.336 957.772 327.499 950.5 375.543 921.664L445.393 890.457C490.741 873.851 509.571 876.412 538.499 889.192C577.028 910.414 587.499 931.5 649.206 964.222C729.486 1006.79 793.126 956.041 817.513 889.192C874.807 742.915 814.513 422.979 650.33 310.48C516.053 226.594 403.003 247.226 270.547 382.715Z"
            fill="#3A3A3A"
            filter={`url(#${p("f0")})`}
          />

          {/* Left arm */}
          <g transform={`rotate(${leftArmAngle}, 226, 578)`}>
            <path
              d="M257.699 773.068C271.728 736.698 272.986 728.506 287.229 709.133C299.637 692.255 259.841 627.746 226.231 577.586C219.861 568.08 205.392 568.903 199.905 578.945C176.511 621.76 137.043 694.31 143.076 742.936C156.217 848.842 228.428 842.851 257.699 773.068Z"
              fill="#3A3A3A"
              filter={`url(#${p("f4")})`}
            />
          </g>

          {/* Bag — gentle pendulum sway */}
          <g transform={`rotate(${bagSwing}, 809, 562)`}>
            {/* Bag strap */}
            <path d="M809.219 562.116C810.943 564.299 813.643 569.352 814.5 571.999C813.437 573.914 664.398 718.453 662.943 720.135C660.931 722.463 656.504 724.217 655.73 726.823C655.652 725.678 651.798 713.917 651.256 711.76C651.947 712.338 808.372 561.734 809.219 562.116Z" fill="#7F573A"/>
            {/* Main bag body */}
            <path d="M190.806 491C194.519 504.309 205.784 521.535 214.752 532.161C267.81 595.038 336.55 644.105 407.584 684.151C424.994 693.869 442.737 702.972 460.779 711.456C466.01 713.902 471.508 716.101 476.821 718.438C486.209 722.572 497.576 729.565 507.585 731.015C508.887 730.566 509.441 730.354 510.051 729.059C510.924 727.206 511.087 724.992 511.928 723.109C513.661 719.228 519.02 715.858 522.76 714.171C545.038 704.118 619.341 691.939 641.08 700.392C646.086 702.337 648.196 705.609 650.735 710.016L651.256 711.761C651.798 713.918 655.652 725.679 655.73 726.824C656.313 732.3 659.858 738.575 660.848 744.412C665.317 770.752 670.972 801.777 653.299 824.737C641.266 840.363 615.91 848.362 596.854 851.092C567.669 855.272 534.037 845.281 522.487 815.613C514.088 801.849 511.086 767.005 510.241 750.61C510.01 749.877 509.856 748.891 509.096 748.514C497.021 742.507 481.675 738.616 470.056 732.016C466.407 730.87 458.211 726.897 454.524 725.132C443.472 719.899 432.571 714.356 421.835 708.504C410.521 702.477 398.821 696.635 387.712 690.298C345.935 666.657 306.427 639.208 269.69 608.311C252.994 594.346 239.89 583.267 224.877 566.974C212.893 553.877 201.927 539.881 192.075 525.112C189.416 521.122 185.689 512.36 183.837 510.089L183 510.239C183.378 502.266 183.782 496.434 190.806 491Z" fill="#C89F7B"/>
            {/* Bag shadow */}
            <path d="M650.736 710.017L651.257 711.761C651.799 713.918 655.653 725.679 655.73 726.825C656.314 732.3 659.858 738.575 660.849 744.412C665.318 770.752 670.973 801.777 653.3 824.737C641.267 840.363 615.911 848.362 596.855 851.092C567.67 855.272 534.038 845.281 522.488 815.613C523.676 816.49 525.015 818.838 525.439 818.926C529.443 819.742 535.002 816.01 538.423 814.421C540.962 816.568 544.646 826.455 547.887 828.829C552.582 832.271 563.243 824.329 565.916 820.33C568.011 817.192 570.56 816.129 573.604 814.214C577.882 811.521 577.134 808.853 575.642 804.673C572.443 794.604 562.887 796.219 555.518 792.044C552.319 790.233 547.804 784.386 544.873 781.842C541.385 778.519 540.358 770.607 537.902 766.856C535.878 763.764 535.019 762.257 533.541 758.79L533.794 758.366C536.039 761.865 538.418 766.164 540.983 769.25L541.39 769.73C542.046 771.531 545.049 774.158 546.576 775.67C557.985 784.845 571.556 786.987 585.328 790.471L589.456 791.033C588.258 787.895 587.448 786.466 588.305 783.277C590.482 782.864 590.808 783.179 592.185 781.935C594.358 772.93 596.953 772.331 605.457 770.855C608.006 772.202 613.202 775.851 615.808 777.569C623.527 776.908 638.254 766.52 644.213 761.489C655.679 751.642 654.993 735.747 652.758 722.17C651.984 717.463 650.39 715.022 650.736 710.017Z" fill="#A78160"/>
            {/* Bag clasp */}
            <path d="M588.303 783.276C590.481 782.863 590.806 783.178 592.184 781.935C594.356 772.929 596.951 772.331 605.455 770.855C608.004 772.202 613.2 775.85 615.806 777.569C617.927 780.536 618.974 782.616 618.226 786.435C616.642 794.542 606.853 800.379 598.943 798.547C593.953 797.396 592.137 794.831 589.454 791.033C588.257 787.895 587.447 786.466 588.303 783.276Z" fill="#252525"/>
            <path d="M546.576 775.671C557.985 784.846 571.556 786.988 585.328 790.472C579.652 790.905 574.29 790.487 568.661 789.765C565.379 789.341 561.968 787.773 559.017 789.429C555.751 788.314 548.196 779.02 546.576 775.671Z" fill="#A2795A"/>
            <path d="M541.391 769.728C542.743 770.972 544.987 773.434 546.463 773.929C545.957 771.519 543.785 769.269 542.061 767.571L542.02 766.761C543.527 767.84 545.9 771.034 548.315 772.866C556.22 778.878 565.034 781.18 574.74 782.145C578.083 782.475 585.901 782.614 588.305 783.275C587.449 786.464 588.259 787.894 589.456 791.031L585.328 790.469C571.556 786.985 557.985 784.844 546.576 775.668C545.049 774.156 542.046 771.529 541.391 769.728Z" fill="#7F573A"/>
            <path d="M523.368 725.967C527.723 733.368 523.136 746.775 517.744 752.942C516.114 752.24 511.562 749.727 510.242 750.609C510.011 749.876 509.857 748.891 509.097 748.514C497.022 742.507 481.676 738.616 470.057 732.016C478.257 731.892 508.598 751.61 517.329 745.836C518.576 745.01 518.465 740.107 518.52 738.585C518.563 737.383 516.716 735.38 515.805 734.291C518.237 735.623 520.09 737.94 522.882 737.527C524.578 735.38 523.15 729.378 523.368 725.967Z" fill="#A2795A"/>
            <path d="M511.691 732.274C514.377 727.898 517.699 720.111 523.366 725.968C523.149 729.379 524.577 735.381 522.881 737.528C520.089 737.941 518.236 735.624 515.803 734.292L511.691 732.274Z" fill="#7F573A"/>
            {/* Bag buckle dots */}
            <path d="M577.638 756.983C587.049 758.165 589.681 765.374 580.713 770.473C574.877 770.525 566.987 760.817 577.638 756.983Z" fill="#090909"/>
            <path d="M616.684 750.234C624.507 752.18 628.052 760.168 619.594 763.657C612.2 762.413 609.455 754.595 616.684 750.234Z" fill="#090909"/>
          </g>

          {/* Right arm */}
          <g transform={`rotate(${rightArmAngle}, 712, 577)`}>
            <path
              d="M680.85 773.156C666.821 736.786 665.563 728.594 651.32 709.221C638.911 692.343 678.707 627.834 712.318 577.674C718.688 568.167 733.157 568.991 738.644 579.033C762.038 621.848 801.506 694.398 795.472 743.024C782.332 848.93 710.12 842.939 680.85 773.156Z"
              fill="#3A3A3A"
              filter={`url(#${p("f12")})`}
            />
          </g>

          {/* Head group: drift (hat outside squash so it isn't distorted) */}
          <g transform={`translate(${headDx}, ${headDy})`}>

            {/* Hat cluster — moves with head drift */}
            <path d="M592.564 204.563C607.021 195.397 635.862 196.42 652.061 200.586C669.129 204.976 688.579 216.966 702.096 228.128C703.73 229.477 708.969 233.276 709.97 234.778C711.752 236.207 713.143 237.274 714.765 238.914C720.623 232.413 730.35 221.154 739.527 229.577C741.362 231.394 741.841 231.772 742.97 234.065C750.161 247.715 736.817 249.238 733.951 257.948C737.268 259.252 738.218 259.71 741.137 261.795C746.103 265.633 751.311 268.925 756.191 272.575C763.67 278.166 773.466 288.52 779.17 296.14C800.143 326.203 810.361 370.412 776.386 395.979C770.172 400.655 761.949 403.064 754.398 404.274C752.549 404.569 748.18 404.866 746.894 405.19C743.4 401.593 732.333 397.104 727.379 394.144C720.883 390.264 715.221 386.898 709.097 382.266C669.009 351.945 633.45 315.951 590.711 289.015C587.406 286.932 580.004 281.64 576.606 280.833C576.388 278.189 573.208 274.172 572.492 271.566C571.48 267.884 572.132 263.761 570.563 260.173C570 258 567 241.5 579.833 213.534C583.673 209.342 587.47 206.909 592.564 204.563Z" fill="#DFB690"/>
            <g filter={`url(#${p("f13")})`}>
              <path d="M592.564 204.562C591.447 206.783 587.524 208.421 585.13 209.576C586.254 212.787 587.824 215.965 588.781 218.827C590.975 225.914 592.128 232.951 593.829 240.197C594.894 244.718 593.07 255.565 595.151 259.258C597.787 263.922 603.83 268.245 608.131 271.297C614.607 275.888 620.783 281.475 627.421 285.8C632.008 288.79 634.424 297.602 639.399 299.206C649.625 302.504 654.929 307.446 663.016 314.071C664.967 315.669 671.953 315.939 673.816 317.558C673.698 320.411 672.629 321.171 673.294 322.927L673.981 323L673.853 321.37C674.461 321.232 680.778 324.934 681.518 325.554C688.109 331.054 690.318 328.802 696.474 330.1C707.059 332.333 722.113 341.057 729.251 327.143C732.065 321.34 732.973 313.831 733.802 307.401C738.505 303.44 738.256 298.237 743.76 298.087C745.841 298.03 748.248 300.127 749.389 301.686C754.916 309.32 753.163 308.655 761.794 311.088C770.185 313.453 769.519 312.624 771.375 303.286C772.156 299.334 774.41 297.646 778.273 297L779.17 296.14C800.143 326.202 810.361 370.412 776.386 395.979C770.172 400.655 761.949 403.063 754.398 404.274C752.55 404.568 748.18 404.866 746.894 405.19C743.4 401.593 732.334 397.103 727.379 394.143C720.883 390.263 715.221 386.898 709.097 382.265C669.009 351.945 633.45 315.951 590.711 289.014C587.407 286.932 580.004 281.64 576.606 280.833C576.388 278.189 573.208 274.172 572.492 271.565C571.48 267.884 572.133 263.761 570.563 260.173C571.5 257 566 241 579.833 213.534C583.673 209.341 587.47 206.909 592.564 204.562Z" fill="#B38C69"/>
            </g>
            <path d="M714.764 238.914C720.623 232.413 730.35 221.154 739.527 229.576C741.362 231.394 741.841 231.772 742.969 234.065C750.16 247.715 736.816 249.238 733.95 257.948C733.119 259.752 732.692 260.091 730.787 261.028C728.436 261.839 726.439 262.467 724.205 263.61C721.927 262.444 720.647 261.881 718.612 260.278L718.537 259.64C719.194 258.757 719.251 258.673 719.709 257.672L719.302 257.339C716.774 255.217 715.494 254.49 713.879 251.509C709.303 245.13 713.844 244.822 714.764 238.914Z" fill="#E3B88E"/>
            <g filter={`url(#${p("f14")})`}>
              <path d="M742.968 234.064C750.16 247.714 736.815 249.238 733.949 257.947C733.118 259.751 732.691 260.09 730.787 261.027C728.435 261.839 726.438 262.467 724.204 263.61C721.927 262.443 720.646 261.88 718.611 260.277L718.537 259.639C719.193 258.757 719.25 258.672 719.708 257.672L719.302 257.339C716.773 255.217 715.493 254.49 713.878 251.509C715.185 252.527 715.936 253.15 717.381 253.953C719.401 253.772 722.21 251.19 725.079 250.082C726.717 250.078 728.002 248.385 729.273 247.162C731.692 244.658 735.552 244.158 738.484 242.4C741.485 240.595 741.469 237.278 742.799 234.424L742.968 234.064Z" fill="#A77754"/>
            </g>
            <path d="M718.611 260.277C720.535 259.611 727.674 258.508 730.285 257.905L730.895 258.524L730.787 261.027C728.435 261.839 726.438 262.467 724.204 263.61C721.927 262.444 720.646 261.88 718.611 260.277Z" fill="#7A5131"/>
            <g filter={`url(#${p("f15")})`}>
              <path d="M725.079 250.081L725.083 247.878C723.54 248.215 720.779 248.396 719.574 247.336C719.959 247.338 723.431 247.301 723.874 247.006C727.857 244.381 735.608 240.411 738.509 237.024C739.362 236.027 738.318 233.461 737.951 232.2C738.389 230.255 737.963 231.146 739.526 229.576C741.361 231.394 741.841 231.771 742.969 234.064L742.799 234.424C741.469 237.278 741.485 240.595 738.484 242.4C735.552 244.158 731.693 244.658 729.273 247.162C728.003 248.385 726.717 250.078 725.079 250.081Z" fill="#BC8860"/>
            </g>
            <path d="M724.164 264.823C719.181 262.105 713.493 261.671 710.938 259.491C710.187 256.534 711.243 255.827 709.399 252.988C707.996 250.836 705.816 248.694 707.118 246.243C708.002 246.639 708.527 248.022 709.155 249.08L709.81 249.136C711.328 246.312 712.47 243.14 713.64 240.137C713.357 238.356 711.458 237.444 709.694 235.483L709.968 234.778C711.75 236.207 713.141 237.274 714.763 238.914C713.843 244.822 709.302 245.13 713.878 251.509C715.493 254.49 716.773 255.217 719.301 257.339L719.707 257.672C719.25 258.673 719.193 258.757 718.536 259.64L718.611 260.278C720.646 261.881 721.926 262.444 724.204 263.61C726.437 262.468 728.435 261.839 730.786 261.028C732.691 260.091 733.118 259.752 733.949 257.948C737.266 259.252 738.216 259.71 741.135 261.795C736.25 261.368 728.569 262.579 724.164 264.823Z" fill="#BC8860"/>

            {/* Head content: squash/stretch */}
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

              {/* Left eye */}
              <g transform={`translate(411, 465) scale(1, ${eyeScaleY}) translate(-411, -465)`}>
                <path d="M411.479 428C419.678 428 423 432 424.408 434.321C431.455 442.807 434.448 450.812 435.286 461.939C436.53 478.451 428.58 501.025 409.175 501.922C402.907 502.212 396.782 499.978 392.176 495.714C372.967 478.168 379.456 428.811 411.479 428Z" fill="#1C170B"/>
                <g filter={`url(#${p("f5")})`}>
                  <path d="M402.589 435.31C405.113 435.115 406.119 435.015 408.226 436.218C409.449 437.699 409.295 438.305 409.367 440.116C410.18 440.625 410.898 441.111 411.694 441.647L411.904 442.956C419.014 456.194 406.034 468.295 397.004 457.028C387.109 457.791 393.027 445.603 396.045 441.344C398.038 438.531 399.869 437.302 402.589 435.31Z" fill="#FAF3EC"/>
                </g>
                <g filter={`url(#${p("f6")})`}>
                  <path d="M402.405 435.12C405.005 434.923 406.041 434.822 408.211 436.033C409.471 437.522 409.312 438.132 409.386 439.954C410.224 440.465 410.964 440.954 411.784 441.493L412 442.811C408.557 441.118 406.625 439.187 402.54 440.654C395.773 443.086 394.268 451.112 396.652 456.966C386.459 457.733 392.555 445.473 395.664 441.189C397.717 438.36 399.602 437.123 402.405 435.12Z" fill="#3A372F"/>
                </g>
              </g>

              {/* Right eye */}
              <g transform={`translate(589, 466) scale(1, ${eyeScaleY}) translate(-589, -466)`}>
                <path d="M589.369 428.706C621.867 428.523 630.994 493.598 594.351 502.663C555.685 504.419 554.456 433.119 589.369 428.706Z" fill="#1C170B"/>
                <g filter={`url(#${p("f7")})`}>
                  <path d="M576.49 452.759C577.096 454.049 577.139 454.759 576.608 455.979C569.333 454.164 573.451 439.586 580.006 437.664C584.199 436.436 587.823 438.013 589.305 442.115C592.618 444.137 594.846 446.01 595.748 450.049C596.354 452.791 595.844 455.661 594.33 458.027C589.037 466.354 580.302 462.46 578.514 452.619C577.655 451.775 577.929 451.624 577.757 450.079L577.59 450.499L577.886 450.8L577.386 452.615L576.49 452.759Z" fill="#FAF3EC"/>
                </g>
                <g filter={`url(#${p("f8")})`}>
                  <path d="M576.06 452.732C576.72 454.041 576.766 454.762 576.188 456C568.275 454.158 572.754 439.363 579.885 437.413C584.446 436.166 588.388 437.766 590 441.93L585.246 442.04C580.159 445.421 579.418 446.592 578.261 452.59C577.327 451.734 577.625 451.58 577.438 450.013L577.257 450.438L577.578 450.743L577.035 452.586L576.06 452.732Z" fill="#312E24"/>
                </g>
                <g filter={`url(#${p("f9")})`}>
                  <path d="M576.49 452.759L575.947 452.886L575.475 452.235C575.11 444.84 575.121 438.674 584.935 442.224C580.259 445.556 579.577 446.709 578.514 452.619C577.655 451.776 577.928 451.624 577.757 450.08L577.59 450.499L577.886 450.8L577.386 452.615L576.49 452.759Z" fill="#534639"/>
                </g>
              </g>

              {/* Left cheek */}
              <path d="M354.002 488.785C366.292 488.07 381.734 490.477 385.001 505.019C386.026 509.579 385.143 514.363 382.556 518.257C378.409 524.432 372.217 526.795 365.337 528.245C353.923 529.158 338.873 527.064 334.774 514.24C333.375 509.718 333.887 504.821 336.192 500.686C339.888 493.968 346.962 490.735 354.002 488.785Z" fill="#F9A6A0"/>
              <g filter={`url(#${p("f10")})`}>
                <path d="M368 494C373.243 494.048 380.362 498.673 380 504C375.832 504.091 367.526 498.087 368 494Z" fill="#FDC3BF"/>
              </g>

              {/* Right cheek */}
              <path d="M626.144 494.285C641.875 485.407 671.146 495.187 664.859 516.522C657.949 539.968 605.952 533.98 615.074 505.471C615.729 503.36 618.569 499.408 620.25 497.867C621.586 496.68 624.464 495.224 626.144 494.285Z" fill="#EF928B"/>
              <g filter={`url(#${p("f11")})`}>
                <path d="M632.012 497C626.769 497.048 619.649 501.673 620.012 507C624.18 507.091 632.486 501.087 632.012 497Z" fill="#FDC3BF"/>
              </g>

              {/* Smirk mouth */}
              <path d="M526.825 509.24C529.058 533.416 509.441 544.063 495.563 543.494C475.913 542.688 463.184 521.332 466.534 509.243C469.883 501.177 484.398 506.216 493.33 507.228C497.024 507.228 500.679 506.536 504.267 505.661C512.377 503.684 525.077 502.133 526.825 509.24Z" fill="#03050D"/>
              <path d="M515.455 529.644C505.491 517.086 486.755 521.664 478.999 530.01C477.283 531.857 477.679 534.691 479.632 536.284C489.719 544.518 503.636 544.538 514.499 536.344C516.626 534.74 517.111 531.73 515.455 529.644Z" fill="#E06B51"/>

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
