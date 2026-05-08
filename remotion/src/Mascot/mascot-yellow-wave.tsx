import React from "react";
import { MascotCharacter, mascotSchema, type MascotProps } from "./lib";

// Variant: waving mascot.
export { mascotSchema };
export type { MascotProps };

export const Mascot: React.FC<MascotProps> = (props) => (
  <MascotCharacter {...props} arm="wave" idPrefix="mascot-wave" />
);
