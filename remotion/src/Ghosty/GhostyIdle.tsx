import React from "react";
import { GhostyCharacter, ghostySchema, type GhostyProps } from "./lib";

// Variant: idle Ghosty (no waving arm).
export const ghostyIdleSchema = ghostySchema;
export type GhostyIdleProps = GhostyProps;

export const GhostyIdle: React.FC<GhostyIdleProps> = (props) => (
  <GhostyCharacter {...props} arm="none" idPrefix="ghosty-idle" />
);
