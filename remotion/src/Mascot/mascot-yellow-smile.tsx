import React from "react";
import {
  AbsoluteFill,
  useCurrentFrame,
  useVideoConfig,
} from "remotion";

/**
 * NewMascotSyicSmile — syicsmile.svg fast energetic animation.
 *
 * - Rapid body bounce (3 Hz)
 * - Fast horizontal wobble (5 Hz)
 * - Head shake side-to-side (2 Hz ±8°)
 * - Both arms flail out-of-phase (3.5 Hz ±28°)
 * - Squinting eyes + big teeth grin stay static (no blink)
 */
export const NewMascotSyicSmile: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps, width, height } = useVideoConfig();
  const p = (k: string) => `nmss-${k}`;

  const t = frame / fps;

  // Fast body bounce
  const bob = Math.sin(t * Math.PI * 3) * 15;
  // Horizontal wobble
  const wobble = Math.sin(t * Math.PI * 5) * 6;
  // Head tilt side-to-side
  const headTilt = Math.sin(t * Math.PI * 2) * 8;
  // Head scale bounce (squash on down, stretch on up)
  const headBounce = Math.abs(Math.sin(t * Math.PI * 3));
  const headScaleX = 1 + headBounce * 0.04;
  const headScaleY = 1 - headBounce * 0.05;

  // Both arms wave fast, out-of-phase
  const leftArmAngle = -20 + Math.sin(t * Math.PI * 3.5) * 28;
  const rightArmAngle = 20 + Math.sin(t * Math.PI * 3.5 + 1.1) * 28;

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
          <filter id={p("f2")} x="423.5" y="239.5" width="153.773" height="66.8594" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="11.25"/>
          </filter>
          <filter id={p("f3")} x="434.977" y="217.946" width="123.535" height="57.3701" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="11.25"/>
          </filter>
          <filter id={p("f4")} x="138.461" y="555.812" width="155.094" height="272.386" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
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
          <filter id={p("f6")} x="366.18" y="492.2" width="15.6352" height="13.601" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.9"/>
          </filter>
          <filter id={p("f7")} x="618.2" y="495.2" width="15.6352" height="13.601" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.9"/>
          </filter>
          <filter id={p("f8")} x="382.973" y="429.157" width="53.9414" height="21.8164" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="1.75"/>
          </filter>
          <filter id={p("f9")} x="537.957" y="428.637" width="54.0859" height="22.3359" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="1.75"/>
          </filter>
        </defs>

        {/* Ground shadow */}
        <ellipse cx={500} cy={978} rx={290} ry={22}
          fill={`url(#${p("ground")})`}
          transform={`scale(${1 - Math.abs(bob) / 600}, 1)`}
          style={{ transformOrigin: "500px 978px" }}
        />

        {/* Everything bobs + wobbles */}
        <g transform={`translate(${wobble}, ${bob})`}>

          {/* Body */}
          <path
            d="M270.549 382.714C175.87 479.647 86.1412 654.573 127.916 829.517C145.273 881.371 165.203 911.976 222.936 941.975C253.338 957.772 327.501 950.5 375.545 921.664L445.395 890.456C490.743 873.851 509.573 876.412 538.501 889.192C577.03 910.413 587.501 931.5 649.208 964.222C729.488 1006.79 793.127 956.041 817.515 889.192C874.809 742.915 814.515 422.978 650.332 310.479C516.055 226.594 403.004 247.226 270.549 382.714Z"
            fill="#F7D145"
            filter={`url(#${p("f0")})`}
          />

          {/* Left arm — fast flail */}
          <g transform={`rotate(${leftArmAngle}, 226, 578)`}>
            <path d="M257.703 773.068C271.731 736.698 272.99 728.506 287.233 709.133C299.641 692.255 259.845 627.746 226.234 577.586C219.865 568.08 205.396 568.903 199.909 578.945C176.514 621.76 137.047 694.31 143.08 742.936C156.221 848.842 228.432 842.851 257.703 773.068Z" fill="#F7D145" filter={`url(#${p("f4")})`}/>
          </g>

          {/* Right arm — fast flail, out-of-phase */}
          <g transform={`rotate(${rightArmAngle}, 712, 577)`}>
            <path d="M680.852 773.156C666.823 736.786 665.565 728.594 651.322 709.221C638.913 692.343 678.709 627.834 712.32 577.674C718.69 568.167 733.159 568.991 738.646 579.033C762.04 621.848 801.508 694.398 795.474 743.024C782.334 848.93 710.122 842.939 680.852 773.156Z" fill="#F7D145" filter={`url(#${p("f5")})`}/>
          </g>

          {/* Head group — tilt + bounce scale */}
          <g transform={`rotate(${headTilt}, 493, 145)`}>
            <g transform={`translate(493, 145) scale(${headScaleX}, ${headScaleY}) translate(-493, -145)`}>

              {/* Neck shadows */}
              <g opacity={0.4} filter={`url(#${p("f2")})`}>
                <path d="M450.376 270.172C464.042 264.005 502.076 255.372 544.876 270.172C598.376 288.672 415.876 288.172 450.376 270.172Z" fill="#B23C05"/>
              </g>
              <g opacity={0.4} filter={`url(#${p("f3")})`}>
                <path d="M533.499 245.499C524.955 248.602 489.943 257.335 463.185 249.888C429.739 240.578 555.068 236.442 533.499 245.499Z" fill="#B23C05"/>
              </g>

              {/* Head circle */}
              <circle cx={493} cy={145} r={110} fill="#F7D145" filter={`url(#${p("f1")})`}/>

              {/* Dark swoosh / brow accent */}
              <path fillRule="evenodd" clipRule="evenodd" d="M692.738 251.916C682.65 247.681 666.943 248.507 659.207 256.56C635.871 244.043 604.514 235.468 578.77 249.543C551.165 267.02 569.907 304.778 585.904 316.433C585.904 316.433 588.624 314.561 607.542 288.671C640.404 293.681 672.003 312.482 687.637 342.407L683.75 350.989C683.089 352.451 682.392 353.902 681.761 355.379C681.358 356.325 681.082 357.3 681.55 358.277C683.099 361.506 687.943 358.567 691.482 357.829C692.428 357.937 678.722 382.701 679.625 383.003C674.738 385.553 682.282 387.826 674.111 392.265C682.149 398.213 692.428 399.664 701.971 401.281C751.773 411.833 759.037 358.285 727.501 314.837C721.41 307.233 714.263 300.449 706.506 294.625C705.447 293.831 706.485 292.02 707.56 292.826C712.714 296.688 717.56 300.944 722.055 305.557L722.573 303.648C723.439 300.434 724.282 297.072 724.508 293.984C724.772 291.873 724.883 289.571 724.596 287.742C725.042 282.488 715.638 261.533 692.738 251.916ZM690.835 353.873L690.996 354.035C691.12 354.161 691.263 354.266 691.419 354.347C691.168 354.383 690.918 354.428 690.668 354.479C690.707 354.345 690.746 354.21 690.782 354.075L690.835 353.873Z" fill="#272727"/>

              {/* Left cheek */}
              <path d="M354.002 488.785C366.292 488.07 381.734 490.477 385.001 505.019C386.026 509.579 385.143 514.363 382.556 518.257C378.409 524.432 372.217 526.795 365.337 528.245C353.923 529.158 338.873 527.064 334.774 514.24C333.375 509.718 333.887 504.821 336.192 500.686C339.888 493.968 346.962 490.735 354.002 488.785Z" fill="#F9A6A0"/>
              <g filter={`url(#${p("f6")})`}><path d="M368 494C373.243 494.048 380.362 498.673 380 504C375.832 504.091 367.526 498.087 368 494Z" fill="#FDC3BF"/></g>

              {/* Right cheek */}
              <path d="M626.148 494.285C641.879 485.407 671.15 495.187 664.863 516.522C657.953 539.968 605.956 533.98 615.078 505.471C615.733 503.36 618.573 499.408 620.253 497.867C621.59 496.68 624.468 495.224 626.148 494.285Z" fill="#EF928B"/>
              <g filter={`url(#${p("f7")})`}><path d="M632.016 497C626.773 497.048 619.653 501.673 620.016 507C624.184 507.091 632.49 501.087 632.016 497Z" fill="#FDC3BF"/></g>

              {/* Mouth + teeth */}
              <path d="M515.749 507.397C519.998 507.234 538.649 506.643 545.331 507.311C546.65 507.442 547.747 508.378 547.63 509.699C547.373 512.592 545.091 516.427 543.813 518.527C533.184 535.98 516.652 554.046 488.676 547.702C471.561 541.942 458.869 526.116 453.378 511.966C452.629 510.035 454.165 508.062 456.236 508.13C473.7 508.71 499.217 507.924 515.749 507.397Z" fill="black"/>
              <path d="M490.07 521.18L505.078 521.036C505.245 529.415 505.685 537.783 506.398 546.146C501.76 546.481 496.508 547.084 492.239 545.631C489.714 543.631 490.149 525.318 490.07 521.18Z" fill="white"/>
              <path d="M508.566 521.074L523.143 521.048C523.331 526.118 524.062 531.719 524.598 536.816C522.955 538.197 520.733 539.639 518.899 540.907C516.152 542.504 513.002 543.839 510.018 545.173C509.073 537.393 508.708 528.875 508.566 521.074Z" fill="white"/>
              <path d="M481.53 521.341L487.032 521.279C486.99 529.024 487.137 536.769 487.472 544.508L483.291 542.468C479.283 540.185 477.805 539.103 474.33 536.408C473.782 531.441 473.376 526.463 473.113 521.475L481.53 521.341Z" fill="white"/>
              <path d="M543.504 509.521L544.044 510.015C543.32 512.452 541.268 515.776 539.931 518.146C535.618 518.09 530.768 518.198 526.413 518.224L525.582 509.696L543.504 509.521Z" fill="white"/>
              <path d="M507.625 509.845C512.742 509.752 517.861 509.7 522.98 509.695C523.165 512.514 523.13 515.606 523.174 518.445L508.489 518.62C508.04 515.941 507.865 512.591 507.625 509.845Z" fill="white"/>
              <path d="M494.661 510.03C498.052 509.922 501.187 509.947 504.582 509.983C504.536 512.596 505.42 515.904 504.15 517.893C501.833 519.13 501.414 518.707 498.076 518.779L490.023 518.944C490.023 516.089 488.744 513.054 489.929 510.813C491.814 509.731 491.996 510.076 494.661 510.03Z" fill="white"/>
              <path d="M472.246 510.2L486.866 510.138L486.957 519.001L472.789 519.093C472.666 516.125 472.485 513.157 472.246 510.2Z" fill="white"/>
              <path d="M456.722 511.656C456.398 510.989 456.883 510.211 457.624 510.211L469.544 510.206L469.655 519.012L460.716 519.564C459.329 517.087 457.999 514.284 456.722 511.656Z" fill="white"/>
              <path d="M529.507 520.841L538.382 520.676C535.833 524.52 534.176 526.803 531.104 530.399C529.819 531.857 529.909 532.234 528.031 532.95C525.949 531.265 525.821 522.716 526.983 521.067L529.507 520.841Z" fill="white"/>
              <path d="M461.785 521.458L470.14 521.494C470.129 525.08 470.476 528.961 470.709 532.562C468.163 531.053 463.486 523.926 461.785 521.458Z" fill="white"/>

              {/* Left eye (squinting) */}
              <path d="M439.224 428.283C442.798 428.126 450.196 427.529 453.208 428.762L453.446 429.98C446.346 432.518 448.494 433.68 448.715 440.885C449.128 454.367 446.446 470.41 436.967 480.671C424.396 494.271 411.325 490.225 399.073 479.021C387.033 466.513 383.221 449.284 382.474 432.549C376.56 432.588 373.98 432.518 368 431.653C380.835 428.621 423.421 428.833 439.224 428.283Z" fill="black"/>
              <g filter={`url(#${p("f8")})`}><path d="M386.473 432.854L397.275 432.657C397.87 438.927 398.74 442.109 400.914 447.97C407.881 447.499 414.147 446.736 421.075 445.856C417.442 451.537 413.933 457.296 410.55 463.126C417.407 471.414 421.289 474.251 431.241 478.399C432.973 478.965 432.29 478.478 433.411 479.821C426.814 488.291 413.232 486.892 405.866 479.947C392.28 467.148 387.876 450.72 386.473 432.854Z" fill="white"/></g>

              {/* Right eye (squinting) */}
              <path d="M573.186 428.657C578.111 428.515 607.304 426.795 609.546 429.568L608.851 430.66L605.631 431.085C605.294 431.367 604.957 431.658 604.62 431.949C604.634 439.986 604.875 449.697 603.391 457.459C601.521 467.249 596.758 479.584 588.182 485.194C582.201 489.106 575.826 489.53 569.107 488.077C546.617 480.33 539.897 453.688 538.285 432.609C534.318 432.522 532.811 432.562 529 431.556C533.277 428.649 566.048 428.869 573.186 428.657Z" fill="black"/>
              <g filter={`url(#${p("f9")})`}><path d="M541.457 432.404L552.022 432.137C553.107 438.454 553.547 441.023 555.769 447.167C562.562 447.08 569.338 446.483 576.04 445.383L565.486 462.644C572.818 471.263 576.288 473.903 586.773 478.13C587.979 478.531 587.544 478.366 588.543 479.316C582.948 487.534 568.345 486.301 561.295 479.827C547.188 466.887 543.19 450.623 541.457 432.404Z" fill="white"/></g>

            </g>
          </g>

        </g>
      </svg>
    </AbsoluteFill>
  );
};
