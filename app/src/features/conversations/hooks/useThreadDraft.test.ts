import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { DRAFT_SAVE_DEBOUNCE_MS, draftStorageKey, useThreadDraft } from './useThreadDraft';

const store = new Map<string, string>();
vi.mock('../../../store/userScopedStorage', () => ({
  userScopedStorage: {
    getItem: vi.fn(async (key: string) => store.get(key) ?? null),
    setItem: vi.fn(async (key: string, value: string) => {
      store.set(key, value);
    }),
    removeItem: vi.fn(async (key: string) => {
      store.delete(key);
    }),
  },
}));

async function settle() {
  await act(async () => {
    await Promise.resolve();
  });
}

describe('useThreadDraft', () => {
  beforeEach(() => {
    store.clear();
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('restores a stored draft for the thread', async () => {
    store.set(draftStorageKey('t1'), 'half typed');
    const { result } = renderHook(() => useThreadDraft('t1'));
    await settle();
    expect(result.current[0]).toBe('half typed');
  });

  it('persists edits after the debounce and clears on empty', async () => {
    const { result } = renderHook(() => useThreadDraft('t1'));
    await settle();
    act(() => result.current[1]('hello'));
    expect(store.has(draftStorageKey('t1'))).toBe(false);
    await act(async () => {
      vi.advanceTimersByTime(DRAFT_SAVE_DEBOUNCE_MS);
    });
    expect(store.get(draftStorageKey('t1'))).toBe('hello');

    act(() => result.current[1](prev => `${prev}!`));
    expect(result.current[0]).toBe('hello!');
    act(() => result.current[1](''));
    await act(async () => {
      vi.advanceTimersByTime(DRAFT_SAVE_DEBOUNCE_MS);
    });
    expect(store.has(draftStorageKey('t1'))).toBe(false);
  });

  it('keeps drafts apart per thread and saves a pending edit on switch', async () => {
    store.set(draftStorageKey('t2'), 'other thread');
    const { result, rerender } = renderHook(({ id }) => useThreadDraft(id), {
      initialProps: { id: 't1' as string | null },
    });
    await settle();
    act(() => result.current[1]('for t1'));

    rerender({ id: 't2' });
    await settle();
    expect(result.current[0]).toBe('other thread');
    // The t1 edit was still inside the debounce window; the switch flushed it.
    expect(store.get(draftStorageKey('t1'))).toBe('for t1');

    rerender({ id: 't1' });
    await settle();
    expect(result.current[0]).toBe('for t1');
  });

  it('does not delete a stored draft just by mounting', async () => {
    store.set(draftStorageKey('t1'), 'keep me');
    renderHook(() => useThreadDraft('t1'));
    await act(async () => {
      vi.advanceTimersByTime(DRAFT_SAVE_DEBOUNCE_MS * 2);
    });
    expect(store.get(draftStorageKey('t1'))).toBe('keep me');
  });
});
