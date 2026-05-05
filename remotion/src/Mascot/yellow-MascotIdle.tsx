import React from "react";
import { MascotCharacter, mascotSchema, type MascotProps } from "./lib";

// Variant: idle mascot (no arm wave).
export const yellowMascotIdleSchema = mascotSchema;
export type YellowMascotIdleProps = MascotProps;

export const YellowMascotIdle: React.FC<YellowMascotIdleProps> = (props) => (
  <MascotCharacter {...props} arm="steady" idPrefix="mascot-idle" />
);
