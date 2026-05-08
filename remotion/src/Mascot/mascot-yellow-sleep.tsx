import React from "react";
import { z } from "zod";
import { MascotCharacter, mascotSchema } from "./lib";

export const yellowMascotSleepSchema = mascotSchema.extend({
  sleeping: z.boolean().default(true),
});
export type YellowMascotSleepProps = z.infer<typeof yellowMascotSleepSchema>;

// Variant: full-loop sleeping pose with continuous Zzz.
export const YellowMascotSleep: React.FC<YellowMascotSleepProps> = (props) => (
  <MascotCharacter
    {...props}
    arm="steady"
    face="normal"
    talking={false}
    sleeping={true}
    sleepStartSec={0}
    sleepFullSec={0}
    idPrefix="mascot-sleep"
  />
);
