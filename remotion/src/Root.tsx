import type { FC } from "react";
import "./index.css";
import { Composition } from "remotion";
import { Ghosty, ghostySchema } from "./Ghosty/Ghosty";
import { GhostyIdle, ghostyIdleSchema } from "./Ghosty/GhostyIdle";
import { GhostyRecording, ghostyRecordingSchema } from "./Ghosty/GhostyRecording";
import { GhostyLoading, ghostyLoadingSchema } from "./Ghosty/GhostyLoading";
import { GhostyPickup, ghostyPickupSchema } from "./Ghosty/GhostyPickup";
import { Mascot, mascotSchema } from "./Mascot/Mascot";
import { YellowMascotIdle, yellowMascotIdleSchema } from "./Mascot/yellow-MascotIdle";
import { YellowMascotRecording, yellowMascotRecordingSchema } from "./Mascot/yellow-MascotRecording";
import { YellowMascotLoading, yellowMascotLoadingSchema } from "./Mascot/yellow-MascotLoading";
import { YellowMascotPickup, yellowMascotPickupSchema } from "./Mascot/yellow-MascotPickup";
import { YellowMascotTalking, yellowMascotTalkingSchema } from "./Mascot/yellow-MascotTalking";
import { YellowMascotThinking, yellowMascotThinkingSchema } from "./Mascot/yellow-MascotThinking";
import { YellowMascotSleep, yellowMascotSleepSchema } from "./Mascot/yellow-MascotSleep";
import { MascotGreeting, mascotGreetingSchema } from "./Mascot/MascotGreeting";

// Each <Composition> is a character variant rendered with a transparent background.
// Render any of them as alpha MOV via:
//   pnpm render <CompositionId>
// e.g. `pnpm render GhostyWave` → out/GhostyWave.mov

const SHARED = {
  fps: 30,
  width: 1080,
  height: 1080,
} as const;

const GHOSTY_DEFAULTS = {
  bodyColor: "#1a1a1a" as const,
  blushColor: "#f5a3ad" as const,
  recordingColor: "#ff3b30" as const,
  loadingColor: "#ffffff" as const,
};

const YELLOW_DEFAULTS = {
  arm: "steady" as const,
  face: "normal" as const,
  talking: false,
  sleeping: false,
  thinking: false,
  greeting: false,
  recordingColor: "#ff3b30" as const,
  loadingColor: "#ffffff" as const,
};

export const RemotionRoot: FC = () => {
  return (
    <>
      {/* ── Ghosty ─────────────────────────────────────────────────────────── */}
      <Composition
        id="GhostyWave"
        component={Ghosty}
        durationInFrames={180}
        {...SHARED}
        schema={ghostySchema}
        defaultProps={{
          ...GHOSTY_DEFAULTS,
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
          ...GHOSTY_DEFAULTS,
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
          ...GHOSTY_DEFAULTS,
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
          ...GHOSTY_DEFAULTS,
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
          ...GHOSTY_DEFAULTS,
          arm: "none" as const,
          face: "normal" as const,
        }}
      />

      {/* ── Yellow Mascot ──────────────────────────────────────────────────── */}
      <Composition
        id="yellow-MascotWave2"
        component={Mascot}
        durationInFrames={180}
        {...SHARED}
        schema={mascotSchema}
        defaultProps={{
          ...YELLOW_DEFAULTS,
          arm: "wave" as const,
        }}
      />

      <Composition
        id="yellow-MascotIdle"
        component={YellowMascotIdle}
        durationInFrames={180}
        {...SHARED}
        schema={yellowMascotIdleSchema}
        defaultProps={YELLOW_DEFAULTS}
      />

      <Composition
        id="yellow-MascotRecording"
        component={YellowMascotRecording}
        durationInFrames={180}
        {...SHARED}
        schema={yellowMascotRecordingSchema}
        defaultProps={{
          ...YELLOW_DEFAULTS,
          arm: "none" as const,
          face: "recording" as const,
        }}
      />

      <Composition
        id="yellow-MascotLoading"
        component={YellowMascotLoading}
        durationInFrames={Math.round(30 * 1.4 * 3)}
        {...SHARED}
        schema={yellowMascotLoadingSchema}
        defaultProps={{
          ...YELLOW_DEFAULTS,
          arm: "none" as const,
          face: "loading" as const,
        }}
      />

      <Composition
        id="yellow-MascotPickup"
        component={YellowMascotPickup}
        durationInFrames={Math.round(30 * 4)}
        {...SHARED}
        schema={yellowMascotPickupSchema}
        defaultProps={YELLOW_DEFAULTS}
      />

      <Composition
        id="yellow-MascotTalking"
        component={YellowMascotTalking}
        durationInFrames={180}
        {...SHARED}
        schema={yellowMascotTalkingSchema}
        defaultProps={{
          ...YELLOW_DEFAULTS,
          talking: true,
        }}
      />

      <Composition
        id="yellow-MascotThinking"
        component={YellowMascotThinking}
        durationInFrames={Math.round(30 * 6)}
        {...SHARED}
        schema={yellowMascotThinkingSchema}
        defaultProps={{
          ...YELLOW_DEFAULTS,
          thinking: true,
        }}
      />

      <Composition
        id="yellow-MascotSleep"
        component={YellowMascotSleep}
        durationInFrames={Math.round(30 * 10)}
        {...SHARED}
        schema={yellowMascotSleepSchema}
        defaultProps={{
          ...YELLOW_DEFAULTS,
          sleeping: true,
        }}
      />

      <Composition
        id="yellow-MascotGreeting"
        component={MascotGreeting}
        durationInFrames={Math.round(30 * 5)}
        {...SHARED}
        schema={mascotGreetingSchema}
        defaultProps={{
          ...YELLOW_DEFAULTS,
          greeting: true,
        }}
      />
    </>
  );
};
