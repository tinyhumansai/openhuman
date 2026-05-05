import React from 'react';

import { MascotCharacter, type MascotProps, mascotSchema } from './MascotCharacter';

// Variant: idle mascot (no arm wave).
export const yellowMascotIdleSchema = mascotSchema;
export type YellowMascotIdleProps = MascotProps;

export const YellowMascotIdle: React.FC<YellowMascotIdleProps> = props => (
  <MascotCharacter {...props} arm="steady" idPrefix="mascot-idle" />
);
