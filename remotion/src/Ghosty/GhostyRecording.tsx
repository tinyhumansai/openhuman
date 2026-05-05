import React from "react";
import { GhostyCharacter, ghostySchema, type GhostyProps } from "./lib";

// Variant: Ghosty with a pulsing red dot instead of a face,
// signalling that the assistant is currently recording.
export const ghostyRecordingSchema = ghostySchema;
export type GhostyRecordingProps = GhostyProps;

export const GhostyRecording: React.FC<GhostyRecordingProps> = (props) => (
  <GhostyCharacter
    {...props}
    face="recording"
    arm={props.arm ?? "none"}
    idPrefix="ghosty-rec"
  />
);
