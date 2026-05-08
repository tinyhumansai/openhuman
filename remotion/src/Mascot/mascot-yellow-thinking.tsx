import React from "react";
import { z } from "zod";
import { MascotCharacter, mascotSchema } from "./lib";

export const yellowMascotThinkingSchema = mascotSchema.extend({
  thinking: z.boolean().default(true),
});
export type YellowMascotThinkingProps = z.infer<typeof yellowMascotThinkingSchema>;

// Variant: full-loop thinking pose.
export const YellowMascotThinking: React.FC<YellowMascotThinkingProps> = (props) => (
  <MascotCharacter
    {...props}
    arm="steady"
    face="normal"
    talking={false}
    sleeping={false}
    thinking={true}
    thinkInStartSec={0}
    thinkInEndSec={0}
    idPrefix="mascot-thinking"
  />
);
