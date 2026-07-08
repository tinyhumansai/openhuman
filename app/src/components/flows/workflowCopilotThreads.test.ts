import { beforeEach, describe, expect, it, vi } from 'vitest';

import { copilotThreadKey, getCopilotThreadId, setCopilotThreadId } from './workflowCopilotThreads';

// Controllable active-user id so tests can exercise the `${userId}:` scoping
// (#900/#983 convention) without pulling in the real module's boot-priming.
const userScopedState = vi.hoisted(() => ({ activeUserId: null as string | null }));
vi.mock('../../store/userScopedStorage', () => ({
  getActiveUserId: () => userScopedState.activeUserId,
}));

describe('workflowCopilotThreads', () => {
  beforeEach(() => {
    window.localStorage.clear();
    userScopedState.activeUserId = null;
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

  it('degrades to a no-op when localStorage.setItem throws (write path)', () => {
    const original = window.localStorage.setItem;
    // Simulate private-mode / quota errors on the write path.
    window.localStorage.setItem = () => {
      throw new Error('quota exceeded');
    };
    try {
      expect(() => setCopilotThreadId('flow-1', 'thread-abc')).not.toThrow();
    } finally {
      window.localStorage.setItem = original;
    }
  });

  it('degrades to a no-op when localStorage.removeItem throws (clear path)', () => {
    setCopilotThreadId('flow-1', 'thread-abc');

    const original = window.localStorage.removeItem;
    // Simulate private-mode / quota errors on the clear path.
    window.localStorage.removeItem = () => {
      throw new Error('unavailable');
    };
    try {
      expect(() => setCopilotThreadId('flow-1', null)).not.toThrow();
    } finally {
      window.localStorage.removeItem = original;
    }
  });

  describe('user scoping (#900/#983 convention)', () => {
    it('namespaces the storage key with the active user id', () => {
      userScopedState.activeUserId = 'user-a';
      setCopilotThreadId('flow-1', 'thread-abc');
      expect(
        window.localStorage.getItem(`user-a:copilot-thread:${copilotThreadKey('flow-1')}`)
      ).toBe('thread-abc');
    });

    it("keeps two users' thread ids for the same flow isolated", () => {
      userScopedState.activeUserId = 'user-a';
      setCopilotThreadId('flow-1', 'thread-a');

      userScopedState.activeUserId = 'user-b';
      setCopilotThreadId('flow-1', 'thread-b');
      expect(getCopilotThreadId('flow-1')).toBe('thread-b');

      // Switching back to user-a must not see user-b's thread id — an
      // identity flip (or a "clear my data" scoped to one user's `${userId}:*`
      // keys) must never leak or destroy the other user's mapping.
      userScopedState.activeUserId = 'user-a';
      expect(getCopilotThreadId('flow-1')).toBe('thread-a');
    });

    it('falls back to an unscoped key when no user is active yet (pre-login)', () => {
      userScopedState.activeUserId = null;
      setCopilotThreadId('flow-1', 'thread-abc');
      expect(window.localStorage.getItem(`copilot-thread:${copilotThreadKey('flow-1')}`)).toBe(
        'thread-abc'
      );
    });
  });
});
