import { describe, expect, it } from 'vitest';

import { isComposerInteractionBlocked } from '../Conversations';

describe('isComposerInteractionBlocked', () => {
  it('blocks composer interaction while the welcome agent loader is visible', () => {
    expect(
      isComposerInteractionBlocked({ activeThreadId: null, welcomePending: true, rustChat: true })
    ).toBe(true);
  });

  it('blocks composer interaction while a thread is actively running', () => {
    expect(
      isComposerInteractionBlocked({
        activeThreadId: 'thread-1',
        welcomePending: false,
        rustChat: true,
      })
    ).toBe(true);
  });

  it('allows composer interaction when chat is idle and ready', () => {
    expect(
      isComposerInteractionBlocked({ activeThreadId: null, welcomePending: false, rustChat: true })
    ).toBe(false);
  });

  it('blocks composer interaction when rust chat is unavailable', () => {
    expect(
      isComposerInteractionBlocked({ activeThreadId: null, welcomePending: false, rustChat: false })
    ).toBe(true);
  });
});
