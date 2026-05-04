import React from "react";
import { GhostyCharacter, ghostySchema, type GhostyProps } from "./lib";

// Variant: waving Ghosty.
export { ghostySchema };
export type { GhostyProps };

export const Ghosty: React.FC<GhostyProps> = (props) => (
  <GhostyCharacter {...props} arm="wave" idPrefix="ghosty-wave" />
);
