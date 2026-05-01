import type { FC } from "react";
import "./index.css";
import { Composition } from "remotion";
import { Ghosty, ghostySchema } from "./Ghosty/Ghosty";
import { GhostyIdle, ghostyIdleSchema } from "./Ghosty/GhostyIdle";
import { GhostyRecording, ghostyRecordingSchema } from "./Ghosty/GhostyRecording";
import { GhostyLoading, ghostyLoadingSchema } from "./Ghosty/GhostyLoading";
import { GhostyPickup, ghostyPickupSchema } from "./Ghosty/GhostyPickup";

// Each <Composition> is a Ghosty variant rendered with a transparent background.
// Render any of them as alpha MOV via:
//   pnpm render <CompositionId>
// e.g. `pnpm render GhostyWave` → out/GhostyWave.mov

const SHARED = {
  fps: 30,
  width: 1080,
  height: 1080,
} as const;

const SHARED_DEFAULTS = {
  bodyColor: "#1a1a1a" as const,
  blushColor: "#f5a3ad" as const,
  recordingColor: "#ff3b30" as const,
  loadingColor: "#ffffff" as const,
};

export const RemotionRoot: FC = () => {
  return (
    <>
      <Composition
        id="GhostyWave"
        component={Ghosty}
        durationInFrames={180}
        {...SHARED}
        schema={ghostySchema}
        defaultProps={{
          ...SHARED_DEFAULTS,
          arm: "wave" as const,
          face: "normal" as const,
        }}
      />

      <Composition
        id="GhostyIdle"
        component={GhostyIdle}
        durationInFrames={180}
        {...SHARED}
        schema={ghostyIdleSchema}
        defaultProps={{
          ...SHARED_DEFAULTS,
          arm: "none" as const,
          face: "normal" as const,
        }}
      />

      <Composition
        id="GhostyRecording"
        component={GhostyRecording}
        durationInFrames={180}
        {...SHARED}
        schema={ghostyRecordingSchema}
        defaultProps={{
          ...SHARED_DEFAULTS,
          arm: "none" as const,
          face: "recording" as const,
        }}
      />

      <Composition
        id="GhostyLoading"
        component={GhostyLoading}
        durationInFrames={Math.round(30 * 1.4 * 3)}
        {...SHARED}
        schema={ghostyLoadingSchema}
        defaultProps={{
          ...SHARED_DEFAULTS,
          arm: "none" as const,
          face: "loading" as const,
        }}
      />

      <Composition
        id="GhostyPickup"
        component={GhostyPickup}
        durationInFrames={Math.round(30 * 4)}
        {...SHARED}
        schema={ghostyPickupSchema}
        defaultProps={{
          ...SHARED_DEFAULTS,
          arm: "none" as const,
          face: "normal" as const,
        }}
      />
    </>
  );
};
