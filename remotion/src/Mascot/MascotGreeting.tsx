import React from "react";
import { z } from "zod";
import { MascotCharacter, mascotSchema } from "./lib";

export const mascotGreetingSchema = mascotSchema.extend({
  greeting: z.boolean().default(true),
});
export type MascotGreetingProps = z.infer<typeof mascotGreetingSchema>;

// Variant: starts idle, right arm rises up, then waves "hi" continuously.
export const MascotGreeting: React.FC<MascotGreetingProps> = (props) => (
  <MascotCharacter
    {...props}
    arm="steady"
    face="normal"
    talking={false}
    sleeping={false}
    thinking={false}
    greeting={true}
    idPrefix="mascot-greeting"
  />
);
