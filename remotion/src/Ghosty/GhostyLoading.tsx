import React from "react";
import { GhostyCharacter, ghostySchema, type GhostyProps } from "./lib";

// Variant: Ghosty with a spinning loading ring instead of a face.
export const ghostyLoadingSchema = ghostySchema;
export type GhostyLoadingProps = GhostyProps;

export const GhostyLoading: React.FC<GhostyLoadingProps> = (props) => (
  <GhostyCharacter
    {...props}
    face="loading"
    arm={props.arm ?? "none"}
    idPrefix="ghosty-loading"
  />
);
