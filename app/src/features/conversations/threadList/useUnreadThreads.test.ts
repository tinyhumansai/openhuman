import { renderHook } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { useUnreadThreads } from './useUnreadThreads';

describe('useUnreadThreads', () => {
  it('marks a background thread unread when its run finishes', () => {
    const { result, rerender } = renderHook(
      ({ running, selected }: { running: string[]; selected: string | null }) =>
        useUnreadThreads(running, selected),
      { initialProps: { running: ['a', 'b'], selected: 'a' } }
    );
    expect(result.current.size).toBe(0);

    rerender({ running: [], selected: 'a' });
    // 'a' was selected when it finished, so only 'b' is unread.
    expect([...result.current]).toEqual(['b']);
  });

  it('clears unread when the thread is selected', () => {
    const { result, rerender } = renderHook(
      ({ running, selected }: { running: string[]; selected: string | null }) =>
        useUnreadThreads(running, selected),
      { initialProps: { running: ['b'], selected: 'a' as string | null } }
    );
    rerender({ running: [], selected: 'a' });
    expect(result.current.has('b')).toBe(true);

    rerender({ running: [], selected: 'b' });
    expect(result.current.has('b')).toBe(false);
  });

  it('does not mark threads that never ran', () => {
    const { result, rerender } = renderHook(
      ({ running, selected }: { running: string[]; selected: string | null }) =>
        useUnreadThreads(running, selected),
      { initialProps: { running: [] as string[], selected: null as string | null } }
    );
    rerender({ running: [], selected: 'x' });
    expect(result.current.size).toBe(0);
  });
});
