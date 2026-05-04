import React from "react";
import { AbsoluteFill, useCurrentFrame, useVideoConfig } from "remotion";
import { z } from "zod";
import { zColor } from "@remotion/zod-types";
import { ARM_PATH, BODY_PATH, LEFT_LEG_PATH, RIGHT_LEG_PATH, VIEWBOX } from "./paths";
import { GhostyDefs } from "./Defs";
import { RecordingFace } from "./RecordingFace";
import { LoadingFace } from "./LoadingFace";

export const ghostySchema = z.object({
  bodyColor: zColor(),
  blushColor: zColor(),
  /** "wave" = animated waving arm. "none" = no arm at all. */
  arm: z.enum(["wave", "none"]).default("wave"),
  /** "normal" = eyes/mouth/blush. "recording" = pulsing red dot. "loading" = spinning ring. */
  face: z.enum(["normal", "recording", "loading"]).default("normal"),
  /** Recording dot color (only used when face="recording"). */
  recordingColor: zColor().default("#ff3b30"),
  /** Spinner color (only used when face="loading"). */
  loadingColor: zColor().default("#ffffff"),
});

export type GhostyProps = z.infer<typeof ghostySchema>;

/**
 * Ghosty character. Pure presentation — all timing comes from Remotion's
 * useCurrentFrame so the same component renders inside any Composition.
 *
 * Use distinct `idPrefix` values if you ever render two Ghostys in the same SVG
 * tree (the gradient / filter IDs are namespaced via the prefix).
 */
export const GhostyCharacter: React.FC<
  GhostyProps & { idPrefix?: string }
> = ({
  bodyColor,
  blushColor,
  arm = "wave",
  face = "normal",
  recordingColor = "#ff3b30",
  loadingColor = "#ffffff",
  idPrefix = "ghosty",
}) => {
  const frame = useCurrentFrame();
  const { fps, width, height } = useVideoConfig();

  // Gentle bob for the whole character.
  const bob = Math.sin((frame / fps) * Math.PI * 1.2) * 14;

  // Top dot drifts independently and squashes when it presses into the body.
  const dotPhase = (frame / fps) * Math.PI * 1.0;
  const dotDx = Math.sin(dotPhase * 0.7) * 6;
  const dotDy = Math.sin(dotPhase) * 9;
  const press = Math.max(0, Math.sin(dotPhase));
  const dotSquashY = 1 - 0.08 * press;
  const dotSquashX = 1 + 0.05 * press;

  // Wave: oscillating arm rotation — 0 when arm is "none".
  const wave = arm === "wave" ? Math.sin((frame / fps) * Math.PI * 2.4) * 12 : 0;

  // Blink every ~2.6s for ~6 frames — offset so frame 0 is eyes open.
  const blinkPeriod = Math.round(fps * 2.6);
  const blinkOffset = Math.round(blinkPeriod / 2);
  const inBlink = (frame + blinkOffset) % blinkPeriod < 6;
  const blinkScale = inBlink ? 0.12 : 1;

  const size = Math.min(width, height) * 0.85;
  const id = (k: string) => `${idPrefix}-${k}`;
  const bodyFill = `url(#${id("body")})`;
  const dotFill = `url(#${id("dot")})`;

  return (
    <AbsoluteFill style={{ justifyContent: "center", alignItems: "center" }}>
      <svg
        width={size}
        height={size}
        viewBox={`0 0 ${VIEWBOX} ${VIEWBOX}`}
        style={{ overflow: "visible" }}
      >
        <GhostyDefs idPrefix={idPrefix} bodyColor={bodyColor} />

        {/* Ground shadow. */}
        <g
          transform={`translate(500, 970) scale(${1 - bob / 600}, 1)`}
          style={{ transformOrigin: "500px 970px" }}
        >
          <ellipse cx={0} cy={0} rx={260} ry={28} fill={`url(#${id("ground")})`} />
        </g>

        {/* Legs stay planted. */}
        <g filter={`url(#${id("drop")})`}>
          <path d={LEFT_LEG_PATH} fill={bodyFill} />
          <path d={RIGHT_LEG_PATH} fill={bodyFill} />
        </g>

        {/* Body + head-dot bob together; legs stay fixed below. */}
        <g transform={`translate(0, ${bob})`} filter={`url(#${id("drop")})`}>
          {/* Top head-dot — floats and squashes against the body apex. */}
          <g
            transform={
              `translate(${dotDx}, ${dotDy}) ` +
              `translate(520 240) scale(${dotSquashX} ${dotSquashY}) translate(-520 -240)`
            }
          >
            <ellipse cx={520} cy={155} rx={92} ry={88} fill={dotFill} />
            <ellipse cx={490} cy={120} rx={24} ry={14} fill="#ffffff" opacity={0.18} />
          </g>

          {arm !== "none" && (
            <g transform={`rotate(${wave} 820 590)`}>
              <path d={ARM_PATH} fill={bodyFill} />
            </g>
          )}

          <path d={BODY_PATH} fill={bodyFill} />

          <g clipPath={`url(#${id("body-clip")})`}>
            <g filter={`url(#${id("soft")})`}>
              <ellipse cx={340} cy={380} rx={220} ry={160} fill="#ffffff" opacity={0.09} />
              <ellipse cx={720} cy={800} rx={280} ry={170} fill="#000000" opacity={0.45} />
            </g>
            <rect x={0} y={0} width={1000} height={1000} filter={`url(#${id("grain")})`} />
          </g>

          {face === "normal" && (
            <>
              <ellipse cx={360} cy={545} rx={48} ry={22} fill={blushColor} opacity={0.85} />
              <ellipse cx={680} cy={545} rx={48} ry={22} fill={blushColor} opacity={0.85} />

              <g>
                <ellipse cx={415} cy={515} rx={30} ry={40 * blinkScale} fill="#0a0a0a" />
                <ellipse cx={625} cy={515} rx={30} ry={40 * blinkScale} fill="#0a0a0a" />
                {!inBlink && (
                  <>
                    <circle cx={425} cy={501} r={7} fill="#ffffff" />
                    <circle cx={635} cy={501} r={7} fill="#ffffff" />
                  </>
                )}
              </g>

              <path d="M478,570 Q520,617 562,570 Q520,597 478,570 Z" fill="#0a0a0a" />

              <ellipse cx={360} cy={545} rx={56} ry={26} fill={blushColor} opacity={0.18} />
              <ellipse cx={680} cy={545} rx={56} ry={26} fill={blushColor} opacity={0.18} />
            </>
          )}

          {face === "recording" && <RecordingFace frame={frame} fps={fps} color={recordingColor} />}

          {face === "loading" && <LoadingFace frame={frame} fps={fps} color={loadingColor} />}
        </g>
      </svg>
    </AbsoluteFill>
  );
};
