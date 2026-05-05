import React from "react";
import { z } from "zod";
import { MascotCharacter, mascotSchema } from "./lib";

export const yellowMascotSleepSchema = mascotSchema.extend({
  sleeping: z.boolean().default(true),
});
export type YellowMascotSleepProps = z.infer<typeof yellowMascotSleepSchema>;

// Variant: mascot blinks a few times, slowly closes eyes, then floats Zzz.
export const YellowMascotSleep: React.FC<YellowMascotSleepProps> = (props) => (
  <MascotCharacter
    {...props}
    arm="steady"
    face="normal"
    talking={false}
    sleeping={true}
    idPrefix="mascot-sleep"
  />
);
