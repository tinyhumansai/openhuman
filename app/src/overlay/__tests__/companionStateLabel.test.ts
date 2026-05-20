import { describe, expect, it } from 'vitest';

import { companionStateLabel } from '../OverlayApp';

// Mock t simply echoes the key in brackets so we can verify the helper picks
// the correct translation key per state.
const t = (key: string) => `[${key}]`;

describe('companionStateLabel', () => {
  it.each([
    ['listening', '“[overlay.companion.listening]”'],
    ['thinking', '“[overlay.companion.thinking]”'],
    ['speaking', '“[overlay.companion.speaking]”'],
    ['pointing', '“[overlay.companion.pointing]”'],
  ])('routes %s through the matching i18n key', (state, expected) => {
    expect(companionStateLabel(state, t)).toBe(expected);
  });

  it('falls back to raw state (unwrapped through t) in the default branch', () => {
    expect(companionStateLabel('mystery', t)).toBe('“mystery”');
    expect(companionStateLabel('idle', t)).toBe('“idle”');
  });
});
