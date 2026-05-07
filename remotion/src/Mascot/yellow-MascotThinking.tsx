import React from "react";
import { z } from "zod";
import { MascotCharacter, mascotSchema } from "./lib";

export const yellowMascotThinkingSchema = mascotSchema.extend({
  thinking: z.boolean().default(true),
});
export type YellowMascotThinkingProps = z.infer<typeof yellowMascotThinkingSchema>;

// Variant: starts idle, then transitions into a thinking pose —
// right arm raises, head tilts, eyes look up, smile becomes a thoughtful "hmm".
export const YellowMascotThinking: React.FC<YellowMascotThinkingProps> = (props) => (
  <MascotCharacter
    {...props}
    arm="steady"
    face="normal"
    talking={false}
    sleeping={false}
    thinking={true}
    idPrefix="mascot-thinking"
  />
);
