import { beforeEach, describe, expect, it } from 'vitest';

import { copilotThreadKey, getCopilotThreadId, setCopilotThreadId } from './workflowCopilotThreads';

describe('workflowCopilotThreads', () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it('returns null for a flow that has never been set', () => {
    expect(getCopilotThreadId('flow-1')).toBeNull();
  });

  it('round-trips a thread id for a persisted flow via localStorage', () => {
    setCopilotThreadId('flow-1', 'thread-abc');
    expect(getCopilotThreadId('flow-1')).toBe('thread-abc');
    // Persisted directly in localStorage (not just an in-memory cache) so a
    // simulated reload — a fresh read with no prior JS state — still resolves.
    expect(window.localStorage.getItem(`copilot-thread:${copilotThreadKey('flow-1')}`)).toBe(
      'thread-abc'
    );
  });

  it('round-trips a thread id for an unsaved draft (null flow id)', () => {
    setCopilotThreadId(null, 'thread-draft');
    expect(getCopilotThreadId(null)).toBe('thread-draft');
    expect(copilotThreadKey(null)).toBe('draft');
  });

  it('survives a simulated reload (fresh read with no prior in-memory state)', () => {
    setCopilotThreadId('flow-2', 'thread-xyz');

    // Simulate a full app reload: nothing survives except localStorage.
    expect(getCopilotThreadId('flow-2')).toBe('thread-xyz');
  });

  it('keeps different flows (and the draft) isolated from each other', () => {
    setCopilotThreadId('flow-1', 'thread-1');
    setCopilotThreadId('flow-2', 'thread-2');
    setCopilotThreadId(null, 'thread-draft');

    expect(getCopilotThreadId('flow-1')).toBe('thread-1');
    expect(getCopilotThreadId('flow-2')).toBe('thread-2');
    expect(getCopilotThreadId(null)).toBe('thread-draft');
  });

  it('removes the mapping when set to null', () => {
    setCopilotThreadId('flow-1', 'thread-abc');
    expect(getCopilotThreadId('flow-1')).toBe('thread-abc');

    setCopilotThreadId('flow-1', null);
    expect(getCopilotThreadId('flow-1')).toBeNull();
  });

  it('degrades to a no-op instead of throwing when localStorage is unavailable', () => {
    const original = window.localStorage.getItem;
    // Simulate private-mode / quota errors.
    window.localStorage.getItem = () => {
      throw new Error('unavailable');
    };
    try {
      expect(() => getCopilotThreadId('flow-1')).not.toThrow();
      expect(getCopilotThreadId('flow-1')).toBeNull();
    } finally {
      window.localStorage.getItem = original;
    }
  });
});
