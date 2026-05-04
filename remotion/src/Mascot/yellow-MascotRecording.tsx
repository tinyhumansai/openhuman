import React from "react";
import { MascotCharacter, mascotSchema, type MascotProps } from "./lib";

// Variant: mascot with a pulsing red dot instead of a face.
export const yellowMascotRecordingSchema = mascotSchema;
export type YellowMascotRecordingProps = MascotProps;

export const YellowMascotRecording: React.FC<YellowMascotRecordingProps> = (props) => (
  <MascotCharacter
    {...props}
    face="recording"
    arm={props.arm ?? "none"}
    idPrefix="mascot-rec"
  />
);
