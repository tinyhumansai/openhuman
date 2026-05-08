import type { FC } from 'react';
import { z } from 'zod';

import { useVideoConfig } from './frameContext';
import { MascotCharacter, mascotSchema } from './MascotCharacter';

export const yellowMascotThinkingSchema = mascotSchema.extend({
  thinking: z.boolean().default(true),
});
export type YellowMascotThinkingProps = z.infer<typeof yellowMascotThinkingSchema>;

// Variant: starts idle, ramps into a thinking pose, holds, then ramps back to idle —
// so the first and last frames match and the composition loops cleanly.
// Ramp-in starts almost immediately so the action reads quickly.
export const YellowMascotThinking: FC<YellowMascotThinkingProps> = props => {
  const { fps, durationInFrames } = useVideoConfig();
  const totalSec = durationInFrames / fps;

  // Quick entrance so the pose is visible early in the loop.
  const thinkInStartSec = 0.15;
  const thinkInEndSec = 0.85;
  // Exit ramps back to idle and finishes exactly on the last frame.
  const thinkOutEndSec = totalSec;
  const thinkOutStartSec = Math.max(thinkInEndSec + 0.2, totalSec - 0.85);

  return (
    <MascotCharacter
      {...props}
      arm="steady"
      face="normal"
      talking={false}
      sleeping={false}
      thinking={true}
      idPrefix="mascot-thinking"
      thinkInStartSec={thinkInStartSec}
      thinkInEndSec={thinkInEndSec}
      thinkOutStartSec={thinkOutStartSec}
      thinkOutEndSec={thinkOutEndSec}
    />
  );
};
