import React from "react";
import {
  AbsoluteFill,
  useCurrentFrame,
  useVideoConfig,
} from "remotion";

/**
 * BlackMascotLaughing — black variant of YellowMascotLaughing.
 *
 * Both arms wave out-of-phase with laughter.
 * Body bounces rapidly + shakes horizontally.
 * Head tilts side-to-side.
 * Happy ^^ eyes, open mouth + tongue.
 * Filter matrices from BlackIdelmascot.svg.
 */
export const BlackMascotLaughing: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps, width, height } = useVideoConfig();
  const p = (k: string) => `bmla-${k}`;

  // ── Body bounce — rapid 3 Hz laughter bounce ────────────────────────────
  const bob = Math.sin((frame / fps) * Math.PI * 3.0) * 18;

  // ── Horizontal body wobble — small 5 Hz side shake ──────────────────────
  const wobble = Math.sin((frame / fps) * Math.PI * 5.0) * 5;

  // ── Head drift + squash ─────────────────────────────────────────────────
  const dotPhase = (frame / fps) * Math.PI;
  const headDx = Math.sin(dotPhase * 2.5) * 8 + wobble * 0.5;
  const headDy = Math.sin(dotPhase * 3.0) * 9;
  const press = Math.max(0, Math.sin(dotPhase * 3.0));
  const headSquashY = 1 - 0.1 * press;
  const headSquashX = 1 + 0.07 * press;

  // ── Head tilt — side-to-side laugh ──────────────────────────────────────
  const headTilt = Math.sin((frame / fps) * Math.PI * 2.0) * 9;

  // ── Both arms shake with laughter (opposite phases) ─────────────────────
  const leftArmAngle = -15 + Math.sin((frame / fps) * Math.PI * 3.5) * 25;
  const rightArmAngle = 15 + Math.sin((frame / fps) * Math.PI * 3.5 + 1.1) * 25;

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
          <filter id={p("f0")} x="90.3867" y="238.634" width="765.266" height="762.131" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
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

          {/* Head circle */}
          <filter id={p("f1")} x="379.002" y="22" width="233" height="237" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
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

          {/* Neck shadows */}
          <filter id={p("f2")} x="423.502" y="239.5" width="153.77" height="66.86" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="11.25"/>
          </filter>
          <filter id={p("f3")} x="434.979" y="217.947" width="123.535" height="57.3708" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="11.25"/>
          </filter>

          {/* Left arm */}
          <filter id={p("f4")} x="138.459" y="555.812" width="155.094" height="272.386" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
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

          {/* Right arm */}
          <filter id={p("f5")} x="645" y="555.9" width="155.094" height="272.386" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
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

          {/* Cheek highlights */}
          <filter id={p("f6")} x="366.18" y="492.2" width="15.6312" height="13.601" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
            <feFlood floodOpacity="0" result="BackgroundImageFix"/>
            <feBlend mode="normal" in="SourceGraphic" in2="BackgroundImageFix" result="shape"/>
            <feGaussianBlur stdDeviation="0.9"/>
          </filter>
          <filter id={p("f7")} x="618.202" y="495.2" width="15.6312" height="13.601" filterUnits="userSpaceOnUse" colorInterpolationFilters="sRGB">
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

        {/* ── Everything bobs + wobbles ─────────────────────────────────────── */}
        <g transform={`translate(${wobble}, ${bob})`}>

          {/* ── Body ────────────────────────────────────────────────────────── */}
          <path
            d="M270.545 382.714C175.866 479.647 86.1373 654.573 127.912 829.517C145.269 881.371 165.199 911.976 222.932 941.975C253.334 957.772 327.497 950.5 375.541 921.664L445.391 890.456C490.739 873.851 509.569 876.412 538.497 889.192C577.026 910.413 587.497 931.5 649.204 964.222C729.484 1006.79 793.124 956.041 817.511 889.192C874.805 742.915 814.511 422.978 650.328 310.479C516.051 226.594 403.001 247.226 270.545 382.714Z"
            fill="#3A3A3A"
            filter={`url(#${p("f0")})`}
          />

          {/* ── Left arm — shakes up with laughter ─────────────────────────── */}
          <g transform={`rotate(${leftArmAngle}, 226, 578)`}>
            <path
              d="M257.699 773.068C271.728 736.698 272.986 728.506 287.229 709.133C299.637 692.255 259.841 627.746 226.231 577.586C219.861 568.08 205.392 568.903 199.905 578.945C176.511 621.76 137.043 694.31 143.076 742.936C156.217 848.842 228.428 842.851 257.699 773.068Z"
              fill="#3A3A3A"
              filter={`url(#${p("f4")})`}
            />
          </g>

          {/* ── Right arm — shakes up with laughter (opposite phase) ─────────── */}
          <g transform={`rotate(${rightArmAngle}, 712, 577)`}>
            <path
              d="M680.848 773.156C666.819 736.786 665.561 728.594 651.318 709.221C638.909 692.343 678.705 627.834 712.316 577.674C718.686 568.167 733.155 568.991 738.642 579.033C762.036 621.848 801.504 694.398 795.471 743.024C782.33 848.93 710.118 842.939 680.848 773.156Z"
              fill="#3A3A3A"
              filter={`url(#${p("f5")})`}
            />
          </g>

          {/* ── Head group: drift + tilt + squash ───────────────────────────── */}
          <g transform={`translate(${headDx}, ${headDy}) rotate(${headTilt}, 493, 145)`}>
            <g transform={`translate(493, 145) scale(${headSquashX}, ${headSquashY}) translate(-493, -145)`}>

              {/* Neck shadows */}
              <g opacity={0.4} filter={`url(#${p("f2")})`}>
                <path d="M450.372 270.172C464.038 264.005 502.072 255.372 544.872 270.172C598.372 288.672 415.872 288.172 450.372 270.172Z" fill="#030100"/>
              </g>
              <g opacity={0.4} filter={`url(#${p("f3")})`}>
                <path d="M533.495 245.499C524.951 248.602 489.939 257.335 463.181 249.888C429.735 240.578 555.064 236.442 533.495 245.499Z" fill="white"/>
              </g>

              {/* Head circle */}
              <circle cx={492.996} cy={145} r={110} fill="#3A3A3A" filter={`url(#${p("f1")})`}/>

              {/* Happy ^^ eyes */}
              <path d="M435.945 461.78C435.95 465.065 432.45 467.564 429.034 466.431C426.95 465.064 426.5 462.935 424.989 460.722C418.079 450.594 409.39 449.424 398.323 453.628C392.683 457.592 392.521 458.972 388.838 464.9C379.031 471.83 374.986 451.985 396.265 443.479C404.335 440.327 413.326 440.509 421.263 443.978C428.823 447.378 434.402 453.5 435.945 461.78Z" fill="#1C170B"/>
              <path d="M618.676 468.507C618.68 471.792 615.18 474.291 611.764 473.157C609.68 471.791 609.23 469.661 607.72 467.449C600.809 457.321 592.12 456.15 581.054 460.355C575.413 464.318 575.251 465.698 571.568 471.627C561.762 478.556 557.717 458.712 578.996 450.206C587.066 447.053 596.056 447.236 603.994 450.705C611.553 454.104 617.133 460.226 618.676 468.507Z" fill="#1C170B"/>

              {/* Left cheek */}
              <path d="M353.998 488.785C366.288 488.07 381.73 490.477 384.997 505.019C386.022 509.579 385.139 514.363 382.552 518.257C378.405 524.432 372.213 526.795 365.333 528.245C353.919 529.158 338.869 527.064 334.77 514.24C333.371 509.718 333.883 504.821 336.188 500.686C339.884 493.968 346.958 490.735 353.998 488.785Z" fill="#F9A6A0"/>
              <g filter={`url(#${p("f6")})`}>
                <path d="M367.996 494C373.239 494.048 380.359 498.673 379.996 504C375.828 504.091 367.522 498.087 367.996 494Z" fill="#FDC3BF"/>
              </g>

              {/* Right cheek */}
              <path d="M626.144 494.285C641.875 485.407 671.146 495.187 664.859 516.522C657.949 539.968 605.952 533.98 615.074 505.471C615.729 503.36 618.569 499.408 620.25 497.867C621.586 496.68 624.464 495.224 626.144 494.285Z" fill="#EF928B"/>
              <g filter={`url(#${p("f7")})`}>
                <path d="M632.012 497C626.769 497.048 619.649 501.673 620.012 507C624.18 507.091 632.486 501.087 632.012 497Z" fill="#FDC3BF"/>
              </g>

              {/* Open mouth + tongue */}
              <path d="M526.559 506.037C529.118 505.857 530.578 506.352 532.949 507.18C540.011 509.647 541.161 518.064 538.561 524.272C535.04 532.678 527.164 538.441 518.947 541.959C504.589 548.106 488.023 546.785 473.761 541.057C468.46 538.97 460.68 534.705 459.795 528.638C455.511 499.213 487.412 516.413 501.358 514.55C509.779 513.426 518.469 508.037 526.559 506.037Z" fill="black"/>
              <path d="M514.571 529.318C521.129 529.165 521.058 531.475 521.47 537.347C509.407 544.777 496.626 543.843 483.423 541.736C481.545 541.195 480.599 541.096 479.149 539.784C473.781 523.686 495.949 536.217 500.65 535.608C504.12 535.159 510.898 531.045 514.571 529.318Z" fill="#E06B51"/>

            </g>
          </g>
          {/* end head group */}

        </g>
        {/* end bob group */}

      </svg>
    </AbsoluteFill>
  );
};
