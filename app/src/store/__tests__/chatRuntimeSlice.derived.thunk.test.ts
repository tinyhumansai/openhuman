import { configureStore } from '@reduxjs/toolkit';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { DerivedDisplayItem, DerivedTranscriptPage } from '../../types/derivedTranscript';
import reducer, {
  fetchAndHydrateDerivedTranscript,
  setStreamingAssistantForThread,
} from '../chatRuntimeSlice';

const { mockThreadApi, flag } = vi.hoisted(() => ({
  mockThreadApi: { getDerivedTranscript: vi.fn(), getTurnStateHistory: vi.fn() },
  flag: { enabled: true },
}));

vi.mock('../../services/api/threadApi', () => ({ threadApi: mockThreadApi }));
vi.mock('../../utils/config', async importOriginal => {
  const actual = await importOriginal<typeof import('../../utils/config')>();
  return {
    ...actual,
    get DERIVED_TRANSCRIPT_ENABLED() {
      return flag.enabled;
    },
  };
});

function page(items: DerivedDisplayItem[], overrides: Partial<DerivedTranscriptPage> = {}) {
  return {
    threadId: 'thread-1',
    items,
    total: items.length,
    hasMore: false,
    hasTranscript: true,
    ...overrides,
  } satisfies DerivedTranscriptPage;
}

/** Newest-first page from chronological items (as the RPC returns). */
function newestFirst(chronological: DerivedDisplayItem[]): DerivedDisplayItem[] {
  return [...chronological].reverse();
}

beforeEach(() => {
  flag.enabled = true;
  mockThreadApi.getDerivedTranscript.mockReset();
  mockThreadApi.getTurnStateHistory.mockReset();
});

describe('fetchAndHydrateDerivedTranscript', () => {
  it('hydrates settled-turn trails from the projection, skipping the newest turn', async () => {
    const store = configureStore({ reducer });
    mockThreadApi.getDerivedTranscript.mockResolvedValueOnce(
      page(
        newestFirst([
          { kind: 'turnBoundary', requestId: 'req-old' },
          { kind: 'reasoning', text: 'old thought' },
          { kind: 'toolCall', callId: 'c1', name: 'shell', status: 'success' },
          { kind: 'turnBoundary', requestId: 'req-new' },
          { kind: 'reasoning', text: 'newest thought' },
        ])
      )
    );

    await store.dispatch(fetchAndHydrateDerivedTranscript('thread-1'));

    expect(mockThreadApi.getDerivedTranscript).toHaveBeenCalledWith('thread-1', { limit: 500 });
    expect(mockThreadApi.getTurnStateHistory).not.toHaveBeenCalled();
    const timelines = store.getState().turnTimelinesByThread['thread-1'];
    const transcripts = store.getState().turnTranscriptsByThread['thread-1'];
    // Newest turn (req-new) is skipped — rendered by the live anchor.
    expect(Object.keys(transcripts)).toEqual(['req-old']);
    expect(timelines['req-old']).toHaveLength(1);
    expect(transcripts['req-new']).toBeUndefined();
    expect(timelines['req-new']).toBeUndefined();
  });

  it('falls back to turn_state history when the flag is off', async () => {
    flag.enabled = false;
    const store = configureStore({ reducer });
    mockThreadApi.getTurnStateHistory.mockResolvedValueOnce([]);

    await store.dispatch(fetchAndHydrateDerivedTranscript('thread-1'));

    expect(mockThreadApi.getDerivedTranscript).not.toHaveBeenCalled();
    expect(mockThreadApi.getTurnStateHistory).toHaveBeenCalledWith('thread-1');
  });

  it('falls back to turn_state history when the RPC errors', async () => {
    const store = configureStore({ reducer });
    mockThreadApi.getDerivedTranscript.mockRejectedValueOnce(new Error('boom'));
    mockThreadApi.getTurnStateHistory.mockResolvedValueOnce([]);

    await expect(
      store.dispatch(fetchAndHydrateDerivedTranscript('thread-1'))
    ).resolves.toBeDefined();

    expect(mockThreadApi.getTurnStateHistory).toHaveBeenCalledWith('thread-1');
  });

  it('falls back to turn_state history when the thread has no persisted transcript (legacy)', async () => {
    const store = configureStore({ reducer });
    mockThreadApi.getDerivedTranscript.mockResolvedValueOnce(page([], { hasTranscript: false }));
    mockThreadApi.getTurnStateHistory.mockResolvedValueOnce([]);

    await store.dispatch(fetchAndHydrateDerivedTranscript('thread-1'));

    expect(mockThreadApi.getTurnStateHistory).toHaveBeenCalledWith('thread-1');
    // The legacy fallback ran with empty history — no derived trails installed.
    expect(store.getState().turnTimelinesByThread['thread-1']).toEqual({});
  });

  it('skips a turn that is currently streaming (live-turn requestId skip)', async () => {
    const store = configureStore({ reducer });
    // Seed a live stream on a NON-newest turn (req-mid).
    store.dispatch(
      setStreamingAssistantForThread({
        threadId: 'thread-1',
        streaming: { requestId: 'req-mid', content: 'streaming...', thinking: '' },
      })
    );
    mockThreadApi.getDerivedTranscript.mockResolvedValueOnce(
      page(
        newestFirst([
          { kind: 'turnBoundary', requestId: 'req-old' },
          { kind: 'reasoning', text: 'old thought' },
          { kind: 'turnBoundary', requestId: 'req-mid' },
          { kind: 'reasoning', text: 'mid thought' },
          { kind: 'turnBoundary', requestId: 'req-new' },
          { kind: 'reasoning', text: 'new thought' },
        ])
      )
    );

    await store.dispatch(fetchAndHydrateDerivedTranscript('thread-1'));

    const transcripts = store.getState().turnTranscriptsByThread['thread-1'];
    // req-new skipped (newest), req-mid skipped (streaming), req-old kept.
    expect(Object.keys(transcripts)).toEqual(['req-old']);
    expect(transcripts['req-mid']).toBeUndefined();
    expect(transcripts['req-new']).toBeUndefined();
  });
});
