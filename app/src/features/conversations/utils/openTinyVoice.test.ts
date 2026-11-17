import { describe, expect, it, vi } from 'vitest';

import { openTinyVoice } from './openTinyVoice';

describe('openTinyVoice', () => {
  it('starts live voice on the mascot stage when one is mounted', () => {
    const expandWithVoice = vi.fn();
    const navigate = vi.fn();
    openTinyVoice({ expandWithVoice }, navigate);
    expect(expandWithVoice).toHaveBeenCalledTimes(1);
    expect(navigate).not.toHaveBeenCalled();
  });

  it('falls back to the Human page without a mascot stage', () => {
    const navigate = vi.fn();
    openTinyVoice(undefined, navigate);
    openTinyVoice(null, navigate);
    expect(navigate).toHaveBeenCalledTimes(2);
    expect(navigate).toHaveBeenCalledWith('/human');
  });
});
