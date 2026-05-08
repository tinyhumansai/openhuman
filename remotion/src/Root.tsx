import "./index.css";
import { Composition } from "remotion";
import { Mascot, mascotSchema } from "./Mascot/mascot-yellow-wave";
import {
  YellowMascotIdle,
  yellowMascotIdleSchema,
} from "./Mascot/mascot-yellow-idle";
import {
  YellowMascotPickup,
  yellowMascotPickupSchema,
} from "./Mascot/mascot-yellow-pickup";
import {
  YellowMascotTalking,
  yellowMascotTalkingSchema,
} from "./Mascot/mascot-yellow-talking";
import {
  YellowMascotSleep,
  yellowMascotSleepSchema,
} from "./Mascot/mascot-yellow-sleep";
import {
  YellowMascotThinking,
  yellowMascotThinkingSchema,
} from "./Mascot/mascot-yellow-thinking";
import { NewMascotListening } from "./Mascot/mascot-yellow-listening";
import { NewMascotLove } from "./Mascot/mascot-yellow-love";
import { NewMascotCrying } from "./Mascot/mascot-yellow-crying";
import { NewMascotCelebrate } from "./Mascot/mascot-yellow-celebrate";
import { NewMascotLaughing } from "./Mascot/mascot-yellow-laughing";
import { NewMascotWink } from "./Mascot/mascot-yellow-wink";
import { NewMascotBookReading } from "./Mascot/mascot-yellow-book-reading";
import { NewMascotHatWithBag } from "./Mascot/mascot-yellow-hat-with-bag";
import { NewMascotCupHolding } from "./Mascot/mascot-yellow-cup-holding";
import { NewMascotBobateaHolding } from "./Mascot/mascot-yellow-boba-tea-holding";
import { NewMascotSyicSmile } from "./Mascot/mascot-yellow-smile";
import { NewMascotSyicSmileSlow } from "./Mascot/mascot-yellow-smile-slow";
import { BlackMascotIdle } from "./Mascot/mascot-black-idle";
import { BlackMascotPickup } from "./Mascot/mascot-black-pickup";
import { BlackMascotTalking } from "./Mascot/mascot-black-talking";
import { BlackMascotThinking } from "./Mascot/mascot-black-thinking";
import { BlackMascotSleep } from "./Mascot/mascot-black-sleep";
import { BlackMascotLove } from "./Mascot/mascot-black-love";
import { BlackMascotWave } from "./Mascot/mascot-black-wave";
import { BlackMascotListening } from "./Mascot/mascot-black-listening";
import { BlackMascotCrying } from "./Mascot/mascot-black-crying";
import { BlackMascotWink } from "./Mascot/mascot-black-wink";
import { BlackMascotCelebrate } from "./Mascot/mascot-black-celebrate";
import { BlackMascotHatWithBag } from "./Mascot/mascot-black-hat-with-bag";
import { BlackMascotLaughing } from "./Mascot/mascot-black-laughing";

export const RemotionRoot: React.FC = () => {
  return (
    <>
      <Composition
        id="mascot-yellow-wave"
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
          mascotColor: "yellow" as const,
        }}
      />
      <Composition
        id="mascot-yellow-idle"
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
          mascotColor: "yellow" as const,
        }}
      />
      <Composition
        id="mascot-yellow-pickup"
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
          mascotColor: "yellow" as const,
        }}
      />
      <Composition
        id="mascot-yellow-talking"
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
          mascotColor: "yellow" as const,
        }}
      />
      <Composition
        id="mascot-yellow-thinking"
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
          mascotColor: "yellow" as const,
        }}
      />
      <Composition
        id="mascot-yellow-sleep"
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
          mascotColor: "yellow" as const,
        }}
      />
      <Composition
        id="mascot-yellow-love"
        component={NewMascotLove}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="mascot-yellow-listening"
        component={NewMascotListening}
        durationInFrames={180}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="mascot-yellow-crying"
        component={NewMascotCrying}
        durationInFrames={300}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="mascot-yellow-celebrate"
        component={NewMascotCelebrate}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="mascot-yellow-hat-with-bag"
        component={NewMascotHatWithBag}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="mascot-yellow-book-reading"
        component={NewMascotBookReading}
        durationInFrames={300}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="mascot-yellow-wink"
        component={NewMascotWink}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="mascot-yellow-cup-holding"
        component={NewMascotCupHolding}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="mascot-yellow-boba-tea-holding"
        component={NewMascotBobateaHolding}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="mascot-yellow-smile"
        component={NewMascotSyicSmile}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="mascot-yellow-smile-slow"
        component={NewMascotSyicSmileSlow}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="mascot-yellow-laughing"
        component={NewMascotLaughing}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="mascot-black-idle"
        component={BlackMascotIdle}
        durationInFrames={180}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="mascot-black-pickup"
        component={BlackMascotPickup}
        durationInFrames={Math.round(30 * 4)}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="mascot-black-talking"
        component={BlackMascotTalking}
        durationInFrames={180}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="mascot-black-thinking"
        component={BlackMascotThinking}
        durationInFrames={Math.round(30 * 6)}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="mascot-black-sleep"
        component={BlackMascotSleep}
        durationInFrames={Math.round(30 * 10)}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="mascot-black-love"
        component={BlackMascotLove}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="mascot-black-wave"
        component={BlackMascotWave}
        durationInFrames={180}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="mascot-black-listening"
        component={BlackMascotListening}
        durationInFrames={180}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="mascot-black-crying"
        component={BlackMascotCrying}
        durationInFrames={300}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="mascot-black-wink"
        component={BlackMascotWink}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="mascot-black-celebrate"
        component={BlackMascotCelebrate}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="mascot-black-hat-with-bag"
        component={BlackMascotHatWithBag}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
      <Composition
        id="mascot-black-laughing"
        component={BlackMascotLaughing}
        durationInFrames={270}
        fps={30}
        width={1080}
        height={1080}
        defaultProps={{}}
      />
    </>
  );
};
