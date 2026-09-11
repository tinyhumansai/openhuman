import { renderHook, waitFor } from '@testing-library/react';
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
