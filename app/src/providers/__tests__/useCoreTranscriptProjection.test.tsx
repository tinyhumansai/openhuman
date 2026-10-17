import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { threadApi } from '../../services/api/threadApi';
import type { DerivedDisplayItem, DerivedTranscriptPage } from '../../types/derivedTranscript';
import { useCoreTranscriptProjection } from '../useOpenHumanExternalStore';

vi.mock('../../services/api/threadApi', () => ({ threadApi: { getDerivedTranscript: vi.fn() } }));

const THREAD = 'thread-long';

/** Newest-first items for one settled turn: a tool call after its boundary. */
function turn(requestId: string, callId: string): DerivedDisplayItem[] {
  return [
    { kind: 'toolCall', callId, name: 'web_search_tool', status: 'success', result: 'ok' },
    { kind: 'assistantMessage', content: `answer ${requestId}`, requestId, iteration: 1 },
    { kind: 'turnBoundary', requestId },
  ];
}

/** A sub-agent card attached to turn `a`, which the core updates in place. */
function subagent(
  status: 'running' | 'completed' | 'incomplete' | 'interrupted'
): DerivedDisplayItem {
  return { kind: 'subagent', id: 'sub-1', status, requestId: 'a', items: [] };
}

function page(over: Partial<DerivedTranscriptPage>): DerivedTranscriptPage {
  return { threadId: THREAD, items: [], total: 0, hasMore: false, hasTranscript: true, ...over };
}

describe('useCoreTranscriptProjection paging', () => {
  beforeEach(() => {
    vi.mocked(threadApi.getDerivedTranscript).mockReset();
  });

  it('walks every older page and projects the whole history', async () => {
    vi.mocked(threadApi.getDerivedTranscript)
      .mockResolvedValueOnce(
        page({ items: turn('r-new', 'call-new'), total: 6, hasMore: true, nextCursor: 'c1' })
      )
      .mockResolvedValueOnce(page({ items: turn('r-old', 'call-old'), total: 6, hasMore: false }));

    const { result } = renderHook(() => useCoreTranscriptProjection(THREAD, 'rev-1', undefined));

    await waitFor(() => expect(Object.keys(result.current.timelines)).toHaveLength(2));
    expect(threadApi.getDerivedTranscript).toHaveBeenCalledTimes(2);
    expect(threadApi.getDerivedTranscript).toHaveBeenLastCalledWith(THREAD, {
      limit: 500,
      cursor: 'c1',
    });
    expect(result.current.timelines['r-old']?.[0]).toMatchObject({ id: 'call-old' });
    expect(result.current.timelines['r-new']?.[0]).toMatchObject({ id: 'call-new' });
  });

  it('stops when the core reports no more pages', async () => {
    vi.mocked(threadApi.getDerivedTranscript).mockResolvedValueOnce(
      page({ items: turn('r-only', 'call-only'), total: 3, hasMore: false })
    );

    const { result } = renderHook(() => useCoreTranscriptProjection(THREAD, 'rev-1', undefined));

    await waitFor(() => expect(Object.keys(result.current.timelines)).toEqual(['r-only']));
    expect(threadApi.getDerivedTranscript).toHaveBeenCalledTimes(1);
  });

  it('drops a page that lands after the thread changed', async () => {
    const older = new Promise<DerivedTranscriptPage>(() => {
      /* never resolves: the walk is still in flight when the thread switches */
    });
    vi.mocked(threadApi.getDerivedTranscript)
      .mockResolvedValueOnce(
        page({ items: turn('r-new', 'call-new'), total: 6, hasMore: true, nextCursor: 'c1' })
      )
      .mockReturnValueOnce(older);

    const { result, rerender } = renderHook(
      ({ thread }) => useCoreTranscriptProjection(thread, 'rev-1', undefined),
      { initialProps: { thread: THREAD as string | null } }
    );
    await waitFor(() => expect(Object.keys(result.current.timelines)).toEqual(['r-new']));

    rerender({ thread: null });
    expect(result.current.timelines).toEqual({});
  });
});

describe('useCoreTranscriptProjection identity', () => {
  beforeEach(() => {
    vi.mocked(threadApi.getDerivedTranscript).mockReset();
  });

  /**
   * The projection refetches several times per turn (every change of the last
   * message or the lifecycle). Minting fresh arrays for every turn each time
   * missed the settled-message conversion cache for the whole thread and gave
   * assistant-ui new part objects for turns nothing had happened to.
   */
  it('keeps the same arrays for turns a refetch did not change', async () => {
    const both = [...turn('r-2', 'call-2'), ...turn('r-1', 'call-1')];
    vi.mocked(threadApi.getDerivedTranscript).mockResolvedValue(
      page({ items: both, total: both.length })
    );

    const { result, rerender } = renderHook(
      ({ revision }) => useCoreTranscriptProjection(THREAD, revision, undefined),
      { initialProps: { revision: 'rev-1' } }
    );
    await waitFor(() => expect(Object.keys(result.current.timelines)).toHaveLength(2));
    const first = result.current;

    // A refetch that returns the same turns hands back the same projection.
    rerender({ revision: 'rev-2' });
    await waitFor(() => expect(threadApi.getDerivedTranscript).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(result.current).toBe(first));

    // A refetch that changes ONE turn keeps the other turn's arrays.
    const changed = [
      { kind: 'toolCall', callId: 'call-3', name: 'shell', status: 'success' },
      ...both,
    ] satisfies DerivedDisplayItem[];
    vi.mocked(threadApi.getDerivedTranscript).mockResolvedValue(
      page({ items: changed, total: changed.length })
    );
    rerender({ revision: 'rev-3' });
    await waitFor(() => expect(result.current.timelines['r-2']).toHaveLength(2));
    expect(result.current.timelines['r-1']).toBe(first.timelines['r-1']);
    expect(result.current.timelines['r-2']).not.toBe(first.timelines['r-2']);
  });
});

describe('history refresh stability', () => {
  it('keeps loaded trails visible when a next-turn refetch fails', async () => {
    vi.mocked(threadApi.getDerivedTranscript)
      .mockReset()
      .mockResolvedValueOnce(page({ items: turn('old', 'old-call') }))
      .mockRejectedValueOnce(new Error('temporary RPC failure'));
    const { result, rerender } = renderHook(
      ({ revision }) => useCoreTranscriptProjection(THREAD, revision, undefined),
      { initialProps: { revision: 'before' } }
    );
    await waitFor(() => expect(result.current.timelines.old).toHaveLength(1));
    const previous = result.current;
    await act(async () => rerender({ revision: 'try-now' }));
    expect(result.current).toBe(previous);
  });
});

/**
 * #7731: a refresh fetches only the newest page and merges it with the held
 * older items, unless the history shrank or the page leaves a gap.
 */
describe('useCoreTranscriptProjection incremental refresh', () => {
  const load = async (older: DerivedDisplayItem[] = turn('a', 'a-call')) => {
    const total = 3 + older.length;
    vi.mocked(threadApi.getDerivedTranscript)
      .mockReset()
      .mockResolvedValueOnce(
        page({ items: turn('b', 'b-call'), total, hasMore: true, nextCursor: 'c1' })
      )
      .mockResolvedValueOnce(page({ items: older, total }));
    const hook = renderHook(
      ({ revision }) => useCoreTranscriptProjection(THREAD, revision, undefined),
      { initialProps: { revision: 'rev-1' } }
    );
    await waitFor(() => expect(Object.keys(hook.result.current.timelines)).toHaveLength(2));
    return hook;
  };

  it.each([
    {
      name: 'unchanged history fetches only the newest page',
      pages: [page({ items: turn('b', 'b-call'), total: 6, hasMore: true, nextCursor: 'c1' })],
      turns: ['a', 'b'],
      reusesA: true,
    },
    {
      name: 'an appended turn is merged with the held older items',
      pages: [page({ items: turn('c', 'c-call'), total: 9, hasMore: true, nextCursor: 'c2' })],
      turns: ['a', 'b', 'c'],
      reusesA: true,
    },
    {
      // A detached sub-agent finishes in place, outside the newest page.
      name: 'a held older item still running walks the full history',
      older: [subagent('running'), ...turn('a', 'a-call')],
      pages: [
        page({ items: turn('c', 'c-call'), total: 10, hasMore: true, nextCursor: 'c2' }),
        page({ items: turn('b', 'b-call'), total: 10, hasMore: true, nextCursor: 'c3' }),
        page({ items: [subagent('completed'), ...turn('a', 'a-call')], total: 10 }),
      ],
      turns: ['a', 'b', 'c'],
    },
    {
      // A continuable sub-agent can be resumed later, which rewrites its card
      // in place without growing the history (#7777).
      name: 'a held older incomplete sub-agent walks the full history',
      older: [subagent('incomplete'), ...turn('a', 'a-call')],
      pages: [
        page({ items: turn('c', 'c-call'), total: 10, hasMore: true, nextCursor: 'c2' }),
        page({ items: turn('b', 'b-call'), total: 10, hasMore: true, nextCursor: 'c3' }),
        page({ items: [subagent('completed'), ...turn('a', 'a-call')], total: 10 }),
      ],
      turns: ['a', 'b', 'c'],
    },
    {
      name: 'a held older interrupted sub-agent walks the full history',
      older: [subagent('interrupted'), ...turn('a', 'a-call')],
      pages: [
        page({ items: turn('c', 'c-call'), total: 10, hasMore: true, nextCursor: 'c2' }),
        page({ items: turn('b', 'b-call'), total: 10, hasMore: true, nextCursor: 'c3' }),
        page({ items: [subagent('completed'), ...turn('a', 'a-call')], total: 10 }),
      ],
      turns: ['a', 'b', 'c'],
    },
    {
      name: 'a gap past the held items walks the full history',
      pages: [
        page({ items: turn('d', 'd-call'), total: 12, hasMore: true, nextCursor: 'c3' }),
        page({ items: turn('c', 'c-call'), total: 12 }),
      ],
      turns: ['c', 'd'],
    },
    {
      name: 'a shrunk history walks in full and keeps no stale items',
      pages: [page({ items: turn('b2', 'b2-call'), total: 3 })],
      turns: ['b2'],
    },
  ])('$name', async ({ older, pages, turns, reusesA }) => {
    const { result, rerender } = await load(older);
    const held = result.current.timelines.a;
    for (const next of pages) vi.mocked(threadApi.getDerivedTranscript).mockResolvedValueOnce(next);

    rerender({ revision: 'rev-2' });

    await waitFor(() => expect(Object.keys(result.current.timelines).sort()).toEqual(turns));
    expect(threadApi.getDerivedTranscript).toHaveBeenCalledTimes(2 + pages.length);
    if (reusesA) expect(result.current.timelines.a).toBe(held);
  });

  it('coalesces a burst of revisions into one in-flight fetch', async () => {
    const { result, rerender } = await load();
    let active = 0;
    let maxActive = 0;
    const release: Array<() => void> = [];
    vi.mocked(threadApi.getDerivedTranscript).mockImplementation(() => {
      active += 1;
      maxActive = Math.max(maxActive, active);
      return new Promise(resolve =>
        release.push(() => {
          active -= 1;
          resolve(page({ items: turn('c', 'c-call'), total: 9, hasMore: true, nextCursor: 'c2' }));
        })
      );
    });

    rerender({ revision: 'rev-2' });
    rerender({ revision: 'rev-3' });
    rerender({ revision: 'rev-4' });
    expect(release).toHaveLength(1);
    await act(async () => release[0]());
    await waitFor(() => expect(release).toHaveLength(2));
    await act(async () => release[1]());

    await waitFor(() =>
      expect(Object.keys(result.current.timelines).sort()).toEqual(['a', 'b', 'c'])
    );
    expect(threadApi.getDerivedTranscript).toHaveBeenCalledTimes(4);
    expect(maxActive).toBe(1);
  });

  it('loads the full history of a thread it switches to', async () => {
    vi.mocked(threadApi.getDerivedTranscript)
      .mockReset()
      .mockResolvedValueOnce(page({ items: turn('a', 'a-call'), total: 3 }))
      .mockResolvedValueOnce(
        page({
          threadId: 'other',
          items: turn('y', 'y-call'),
          total: 6,
          hasMore: true,
          nextCursor: 'c1',
        })
      )
      .mockResolvedValueOnce(page({ threadId: 'other', items: turn('x', 'x-call'), total: 6 }));
    const { result, rerender } = renderHook(
      ({ thread }) => useCoreTranscriptProjection(thread, 'rev-1', undefined),
      { initialProps: { thread: THREAD as string } }
    );
    await waitFor(() => expect(Object.keys(result.current.timelines)).toEqual(['a']));

    rerender({ thread: 'other' });

    await waitFor(() => expect(Object.keys(result.current.timelines).sort()).toEqual(['x', 'y']));
    expect(threadApi.getDerivedTranscript).toHaveBeenLastCalledWith('other', {
      limit: 500,
      cursor: 'c1',
    });
  });

  it('honours a queued pass after a refresh fails', async () => {
    const { result, rerender } = await load();
    let fail!: (error: Error) => void;
    vi.mocked(threadApi.getDerivedTranscript)
      .mockReturnValueOnce(
        new Promise((_, reject) => {
          fail = reject;
        })
      )
      .mockResolvedValueOnce(
        page({ items: turn('c', 'c-call'), total: 9, hasMore: true, nextCursor: 'c2' })
      );

    rerender({ revision: 'rev-2' });
    rerender({ revision: 'rev-3' });
    await act(async () => fail(new Error('temporary RPC failure')));

    await waitFor(() =>
      expect(Object.keys(result.current.timelines).sort()).toEqual(['a', 'b', 'c'])
    );
    expect(threadApi.getDerivedTranscript).toHaveBeenCalledTimes(4);
  });

  it('keeps a loop abandoned by a thread round trip from writing', async () => {
    let resolveZombie!: (value: DerivedTranscriptPage) => void;
    vi.mocked(threadApi.getDerivedTranscript)
      .mockReset()
      .mockResolvedValueOnce(
        page({ items: turn('a2', 'a2-call'), total: 6, hasMore: true, nextCursor: 'c1' })
      )
      .mockReturnValueOnce(
        new Promise(resolve => {
          resolveZombie = resolve;
        })
      )
      .mockResolvedValueOnce(page({ threadId: 'other', items: turn('y', 'y-call'), total: 3 }))
      .mockResolvedValueOnce(
        page({ items: turn('a2', 'a2-call'), total: 6, hasMore: true, nextCursor: 'c1' })
      )
      .mockResolvedValueOnce(page({ items: turn('a1', 'a1-call'), total: 6 }));
    const { result, rerender } = renderHook(
      ({ thread }) => useCoreTranscriptProjection(thread, 'rev-1', undefined),
      { initialProps: { thread: THREAD as string } }
    );
    await waitFor(() => expect(threadApi.getDerivedTranscript).toHaveBeenCalledTimes(2));

    rerender({ thread: 'other' });
    await waitFor(() => expect(Object.keys(result.current.timelines)).toEqual(['y']));
    rerender({ thread: THREAD });
    await waitFor(() => expect(Object.keys(result.current.timelines).sort()).toEqual(['a1', 'a2']));

    await act(async () => resolveZombie(page({ items: turn('zombie', 'z-call'), total: 6 })));
    expect(Object.keys(result.current.timelines).sort()).toEqual(['a1', 'a2']);
  });

  it('keeps a merged history within the full-walk cap', async () => {
    // 3,332 filler turns plus the oldest turn: 9,999 items over 20 pages.
    const history: DerivedDisplayItem[] = [];
    for (let i = 3331; i >= 0; i -= 1) history.push(...turn(`f${i}`, `f${i}-call`));
    history.push(...turn('oldest', 'oldest-call'));
    const mock = vi.mocked(threadApi.getDerivedTranscript).mockReset();
    for (let start = 0; start < history.length; start += 500) {
      const more = start + 500 < history.length;
      mock.mockResolvedValueOnce(
        page({
          items: history.slice(start, start + 500),
          total: history.length,
          hasMore: more,
          nextCursor: more ? `c${start + 500}` : undefined,
        })
      );
    }
    const { result, rerender } = renderHook(
      ({ revision }) => useCoreTranscriptProjection(THREAD, revision, undefined),
      { initialProps: { revision: 'rev-1' } }
    );
    await waitFor(() => expect(result.current.timelines.oldest).toBeDefined());

    // Six new items push the merged list five past the 10,000 cap, so the
    // oldest turn's three items fall off its end.
    const fresh = [...turn('n2', 'n2-call'), ...turn('n1', 'n1-call')];
    mock.mockResolvedValueOnce(
      page({
        items: [...fresh, ...history.slice(0, 494)],
        total: history.length + fresh.length,
        hasMore: true,
        nextCursor: 'c-next',
      })
    );
    rerender({ revision: 'rev-2' });

    await waitFor(() => expect(result.current.timelines.n2).toBeDefined());
    expect(result.current.timelines.oldest).toBeUndefined();
    expect(threadApi.getDerivedTranscript).toHaveBeenCalledTimes(21);
  });
});
