import "./index.css";
import { Composition, getStaticFiles } from "remotion";
import { AIVideo, aiVideoSchema } from "./components/AIVideo";
import { MascotWave } from "./components/MascotWave";
import { MascotHi } from "./components/MascotHi";
import { FPS, INTRO_DURATION } from "./lib/constants";
import { getTimelinePath, loadTimelineFromFile } from "./lib/utils";
import { Ghosty, ghostySchema } from "./Ghosty/Ghosty";
import { GhostyIdle, ghostyIdleSchema } from "./Ghosty/GhostyIdle";
import {
  GhostyRecording,
  ghostyRecordingSchema,
} from "./Ghosty/GhostyRecording";
import { GhostyLoading, ghostyLoadingSchema } from "./Ghosty/GhostyLoading";
import { GhostyPickup, ghostyPickupSchema } from "./Ghosty/GhostyPickup";
import { Mascot, mascotSchema } from "./Mascot/Mascot";
import {
  YellowMascotIdle,
  yellowMascotIdleSchema,
} from "./Mascot/yellow-MascotIdle";
import {
  YellowMascotRecording,
  yellowMascotRecordingSchema,
} from "./Mascot/yellow-MascotRecording";
import {
  YellowMascotLoading,
  yellowMascotLoadingSchema,
} from "./Mascot/yellow-MascotLoading";
import {
  YellowMascotPickup,
  yellowMascotPickupSchema,
} from "./Mascot/yellow-MascotPickup";
import {
  YellowMascotTalking,
  yellowMascotTalkingSchema,
} from "./Mascot/yellow-MascotTalking";
import {
  YellowMascotSleep,
  yellowMascotSleepSchema,
} from "./Mascot/yellow-MascotSleep";
import {
  YellowMascotThinking,
  yellowMascotThinkingSchema,
} from "./Mascot/yellow-MascotThinking";
import { MascotGreeting, mascotGreetingSchema } from "./Mascot/MascotGreeting";
import { NewMascotListening } from "./Mascot/NewMascotListening";
import { NewMascotLove } from "./Mascot/NewMascotLove";
import { NewMascotCrying } from "./Mascot/NewMascotCrying";
import { NewMascotCelebrate } from "./Mascot/NewMascotCelebrate";
import { NewMascotLaughing } from "./Mascot/NewMascotLaughing";
import { NewMascotWink } from "./Mascot/NewMascotWink";
import { NewMascotBookReading } from "./Mascot/NewMascotBookReading";
import { NewMascotHatWithBag } from "./Mascot/NewMascotHatWithBag";
import { NewMascotCupHolding } from "./Mascot/NewMascotCupHolding";
import { NewMascotBobateaHolding } from "./Mascot/NewMascotBobateaHolding";
import { NewMascotSyicSmile } from "./Mascot/NewMascotSyicSmile";
import { NewMascotSyicSmileSlow } from "./Mascot/NewMascotSyicSmileSlow";
import { BlackMascotIdle } from "./Mascot/black-MascotIdle";
import { BlackMascotRecording } from "./Mascot/black-MascotRecording";
import { BlackMascotLoading } from "./Mascot/black-MascotLoading";
import { BlackMascotPickup } from "./Mascot/black-MascotPickup";
import { BlackMascotTalking } from "./Mascot/black-MascotTalking";
import { BlackMascotThinking } from "./Mascot/black-MascotThinking";
import { BlackMascotSleep } from "./Mascot/black-MascotSleep";
import { BlackMascotLove } from "./Mascot/black-MascotLove";
import { BlackMascotWave } from "./Mascot/black-MascotWave";
import { BlackMascotListening } from "./Mascot/black-MascotListening";
import { BlackMascotCrying } from "./Mascot/black-MascotCrying";
import { BlackMascotWink } from "./Mascot/black-MascotWink";
import { BlackMascotCelebrate } from "./Mascot/black-MascotCelebrate";
import { BlackMascotHatWithBag } from "./Mascot/black-MascotHatWithBag";
import { BlackMascotLaughing } from "./Mascot/black-MascotLaughing";

export const RemotionRoot: React.FC = () => {
  const staticFiles = getStaticFiles();
  const timelines = staticFiles
    .filter((file) => file.name.endsWith("timeline.json"))
    .map((file) => file.name.split("/")[1]);

  const GHOSTY_SHARED = { fps: 30, width: 1080, height: 1080 } as const;
  const GHOSTY_DEFAULTS = {
    bodyColor: "#1a1a1a" as const,
    blushColor: "#f5a3ad" as const,
    recordingColor: "#ff3b30" as const,
    loadingColor: "#ffffff" as const,
  };

  return (
    <>
      <Composition
        id="GhostyWave"
        component={Ghosty}
        durationInFrames={180}
        {...GHOSTY_SHARED}
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
        {...GHOSTY_SHARED}
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
        {...GHOSTY_SHARED}
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
        {...GHOSTY_SHARED}
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
        {...GHOSTY_SHARED}
        schema={ghostyPickupSchema}
        defaultProps={{
          bodyColor: "#1a1a1a",
          blushColor: "#f5a3ad",
          recordingColor: "#ff3b30",
          loadingColor: "#ffffff",
          arm: "wave" as const,
          face: "normal" as const,
        }}
      />
      <Composition
        id="yellow-MascotWave2"
        component={Mascot}
        durationInFrames={180}
        fps={30}
        width={1080}
        height={1080}
        schema={mascotSchema}
        defaultProps={{
          arm: "wave" as const,
          face: "normal" as const,
          talking: true,
          sleeping: false,
          thinking: false,
          greeting: false,
          recordingColor: "#ff3b30",
          loadingColor: "#ffffff",
        }}
      />
      <Composition
        id="yellow-MascotIdle"
        component={YellowMascotIdle}
        durationInFrames={180}
        fps={30}
        width={1080}
        height={1080}
        schema={yellowMascotIdleSchema}
        defaultProps={{
          arm: "steady" as const,
          face: "normal" as const,
          talking: false,
          sleeping: false,
          thinking: false,
          greeting: false,
          recordingColor: "#ff3b30",
          loadingColor: "#ffffff",
        }}
      />
      <Composition
        id="yellow-MascotRecording"
        component={YellowMascotRecording}
        durationInFrames={180}
        fps={30}
        width={1080}
        height={1080}
        schema={yellowMascotRecordingSchema}
        defaultProps={{
          arm: "steady" as const,
          face: "recording" as const,
          talking: false,
          sleeping: false,
          thinking: false,
          greeting: false,
          recordingColor: "#ff3b30",
          loadingColor: "#ffffff",
        }}
      />
      <Composition
        id="yellow-MascotLoading"
        component={YellowMascotLoading}
        durationInFrames={Math.round(30 * 1.4 * 3)}
        fps={30}
        width={1080}
        height={1080}
        schema={yellowMascotLoadingSchema}
        defaultProps={{
          arm: "steady" as const,
          face: "loading" as const,
          talking: false,
          sleeping: false,
          thinking: false,
          greeting: false,
          recordingColor: "#ff3b30",
          loadingColor: "#ffffff",
        }}
      />
      <Composition
        id="yellow-MascotPickup"
        component={YellowMascotPickup}
        durationInFrames={Math.round(30 * 4)}
        fps={30}
        width={1080}
        height={1080}
        schema={yellowMascotPickupSchema}
        defaultProps={{
          arm: "steady" as const,
          face: "normal" as const,
          talking: false,
          sleeping: false,
          thinking: false,
          greeting: false,
          recordingColor: "#ff3b30",
          loadingColor: "#ffffff",
        }}
      />
      <Composition
        id="yellow-MascotTalking"
        component={YellowMascotTalking}
        durationInFrames={180}
        fps={30}
        width={1080}
        height={1080}
        schema={yellowMascotTalkingSchema}
        defaultProps={{
          arm: "steady" as const,
          face: "normal" as const,
          talking: true,
          sleeping: false,
          thinking: false,
          greeting: false,
          recordingColor: "#ff3b30",
          loadingColor: "#ffffff",
        }}
      />
      <Composition
        id="yellow-MascotThinking"
        component={YellowMascotThinking}
        durationInFrames={Math.round(30 * 6)}
        fps={30}
        width={1080}
        height={1080}
        schema={yellowMascotThinkingSchema}
        defaultProps={{
          arm: "steady" as const,
          face: "normal" as const,
          talking: false,
          sleeping: false,
          thinking: true,
          greeting: false,
          recordingColor: "#ff3b30",
          loadingColor: "#ffffff",
        }}
      />
      <Composition
        id="yellow-MascotSleep"
        component={YellowMascotSleep}
        durationInFrames={Math.round(30 * 10)}
        fps={30}
        width={1080}
        height={1080}
        schema={yellowMascotSleepSchema}
        defaultProps={{
          arm: "wave" as const,
          face: "normal" as const,
          talking: false,
          sleeping: true,
          thinking: false,
          greeting: false,
          recordingColor: "#ff3b30",
          loadingColor: "#ffffff",
        }}
      />
      <Composition
        id="new-MascotLove"
        component={NewMascotLove}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="new-MascotListening"
        component={NewMascotListening}
        durationInFrames={180}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="new-MascotCrying"
        component={NewMascotCrying}
        durationInFrames={300}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="new-MascotCelebrate"
        component={NewMascotCelebrate}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="new-MascotHatWithBag"
        component={NewMascotHatWithBag}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="new-MascotBookReading"
        component={NewMascotBookReading}
        durationInFrames={300}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="new-MascotWink"
        component={NewMascotWink}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="new-MascotCupHolding"
        component={NewMascotCupHolding}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="new-MascotBobateaHolding"
        component={NewMascotBobateaHolding}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="new-MascotSyicSmile"
        component={NewMascotSyicSmile}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="new-MascotSyicSmileSlow"
        component={NewMascotSyicSmileSlow}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="new-MascotLaughing"
        component={NewMascotLaughing}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="black-MascotIdle"
        component={BlackMascotIdle}
        durationInFrames={180}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="black-MascotRecording"
        component={BlackMascotRecording}
        durationInFrames={180}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="black-MascotLoading"
        component={BlackMascotLoading}
        durationInFrames={Math.round(30 * 1.4 * 3)}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="black-MascotPickup"
        component={BlackMascotPickup}
        durationInFrames={Math.round(30 * 4)}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="black-MascotTalking"
        component={BlackMascotTalking}
        durationInFrames={180}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="black-MascotThinking"
        component={BlackMascotThinking}
        durationInFrames={Math.round(30 * 6)}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="black-MascotSleep"
        component={BlackMascotSleep}
        durationInFrames={Math.round(30 * 10)}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="black-MascotLove"
        component={BlackMascotLove}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="black-MascotWave"
        component={BlackMascotWave}
        durationInFrames={180}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="black-MascotListening"
        component={BlackMascotListening}
        durationInFrames={180}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="black-MascotCrying"
        component={BlackMascotCrying}
        durationInFrames={300}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="black-MascotWink"
        component={BlackMascotWink}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="black-MascotCelebrate"
        component={BlackMascotCelebrate}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="black-MascotHatWithBag"
        component={BlackMascotHatWithBag}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="black-MascotLaughing"
        component={BlackMascotLaughing}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      {timelines.map((storyName) => (
        <Composition
          id={storyName}
          component={AIVideo}
          fps={FPS}
          width={1080}
          height={1920}
          schema={aiVideoSchema}
          defaultProps={{
            timeline: null,
          }}
          calculateMetadata={async ({ props }) => {
            const { lengthFrames, timeline } = await loadTimelineFromFile(
              getTimelinePath(storyName),
            );

            return {
              durationInFrames: lengthFrames + INTRO_DURATION,
              props: {
                ...props,
                timeline,
              },
            };
          }}
        />
      ))}
    </>
  );
};
