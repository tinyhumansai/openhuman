import { describe, expect, it } from 'vitest';

import { emptySessionTokenUsage } from '../../../../store/chatRuntimeSlice';
import { contextUsageFromTokenUsage } from './ContextWindowPill';

describe('contextUsageFromTokenUsage', () => {
  it('keeps cached input separate from fresh input', () => {
    expect(
      contextUsageFromTokenUsage({
        ...emptySessionTokenUsage(),
        inputTokens: 1_000,
        cachedTokens: 400,
        outputTokens: 250,
        costUsd: 0.02,
        contextWindow: 128_000,
        lastTurnContextUsed: 900,
      })
    ).toEqual({
      used: 900,
      limit: 128_000,
      input: 600,
      cachedInput: 400,
      output: 250,
      costUsd: 0.02,
    });
  });

  it('preserves an unknown context limit as zero', () => {
    expect(contextUsageFromTokenUsage(emptySessionTokenUsage()).limit).toBe(0);
  });

  it('uses the selected model context window instead of stale turn usage', () => {
    const usage = { ...emptySessionTokenUsage(), contextWindow: 200_000 };
    expect(contextUsageFromTokenUsage(usage, 128_000).limit).toBe(128_000);
    expect(contextUsageFromTokenUsage(usage, null).limit).toBe(0);
  });
});
