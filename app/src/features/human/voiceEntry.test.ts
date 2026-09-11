import { describe, expect, it } from 'vitest';

import { resolveHumanVoiceEntry } from './voiceEntry';

describe('resolveHumanVoiceEntry', () => {
  it('defaults to the realtime control', () => {
    expect(resolveHumanVoiceEntry({ realtimeEnabled: true, showBoth: false })).toBe('realtime');
  });

  it('falls back to push-to-talk when realtime is switched off', () => {
    expect(resolveHumanVoiceEntry({ realtimeEnabled: false, showBoth: false })).toBe(
      'push-to-talk'
    );
  });

  // show-both wins either way: a build that asks to compare the two paths must
  // not have one of them hidden by the other flag's rollback state.
  it('shows both whenever show-both is set, regardless of the realtime flag', () => {
    expect(resolveHumanVoiceEntry({ realtimeEnabled: true, showBoth: true })).toBe('both');
    expect(resolveHumanVoiceEntry({ realtimeEnabled: false, showBoth: true })).toBe('both');
  });
});
