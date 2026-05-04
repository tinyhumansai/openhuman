import React from "react";
import { MascotCharacter, mascotSchema, type MascotProps } from "./lib";

// Variant: mascot with a spinning loading ring instead of a face.
export const yellowMascotLoadingSchema = mascotSchema;
export type YellowMascotLoadingProps = MascotProps;

export const YellowMascotLoading: React.FC<YellowMascotLoadingProps> = (props) => (
  <MascotCharacter
    {...props}
    face="loading"
    arm={props.arm ?? "none"}
    idPrefix="mascot-loading"
  />
);
