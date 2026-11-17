import { describe, expect, it } from 'vitest';

import chatRuntimeReducer, {
  liveTurnStarted,
  turnSettled,
  upsertArtifactFailedForThread,
  upsertArtifactInProgressForThread,
  upsertArtifactReadyForThread,
} from '../chatRuntimeSlice';

type State = ReturnType<typeof chatRuntimeReducer>;

const THREAD = 't-1';
const ready = (artifactId: string) =>
  upsertArtifactReadyForThread({
    threadId: THREAD,
    artifactId,
    kind: 'document',
    title: artifactId,
    path: `/tmp/${artifactId}.md`,
    sizeBytes: 12,
  });
const owner = (state: State, artifactId: string) =>
  state.artifactsByThread[THREAD]?.find(a => a.artifactId === artifactId)?.ownerRequestId;

function apply(state: State, ...actions: Parameters<typeof chatRuntimeReducer>[1][]): State {
  return actions.reduce(chatRuntimeReducer, state);
}

describe('artifact owner turn', () => {
  it('gives an artifact loaded when the thread opens no owner from the live turn', () => {
    const state = apply(
      chatRuntimeReducer(undefined, { type: 'init' }),
      liveTurnStarted({ threadId: THREAD, requestId: 'req-latest' }),
      upsertArtifactReadyForThread({
        threadId: THREAD,
        artifactId: 'old-1',
        kind: 'document',
        title: 'old-1',
        path: '/tmp/old-1.md',
        sizeBytes: 12,
        loaded: true,
      })
    );
    // Its reply comes from the thread's artifact rows; an owner here would pull
    // every older card into the latest reply.
    expect(owner(state, 'old-1')).toBeUndefined();
  });

  it('keeps the owner a live event gave when the same artifact is loaded later', () => {
    const state = apply(
      chatRuntimeReducer(undefined, { type: 'init' }),
      liveTurnStarted({ threadId: THREAD, requestId: 'req-1' }),
      ready('a-1'),
      upsertArtifactReadyForThread({
        threadId: THREAD,
        artifactId: 'a-1',
        kind: 'document',
        title: 'a-1',
        path: '/tmp/a-1.md',
        sizeBytes: 12,
        loaded: true,
      })
    );
    expect(owner(state, 'a-1')).toBe('req-1');
  });

  it('tags an artifact with the turn that was live when it appeared', () => {
    const state = apply(
      chatRuntimeReducer(undefined, { type: 'init' }),
      liveTurnStarted({ threadId: THREAD, requestId: 'req-1' }),
      ready('a-1')
    );
    expect(owner(state, 'a-1')).toBe('req-1');
  });

  it('keeps the owner when the thread list is re-hydrated after the turn ended (refresh)', () => {
    let state = apply(
      chatRuntimeReducer(undefined, { type: 'init' }),
      liveTurnStarted({ threadId: THREAD, requestId: 'req-1' }),
      ready('a-1'),
      turnSettled({ threadId: THREAD, requestId: 'req-1' })
    );
    // ChatFilesChip re-hydrates every listed artifact through the same action.
    state = apply(state, ready('a-1'));
    expect(owner(state, 'a-1')).toBe('req-1');
  });

  it('does not move an older artifact into a newer live turn', () => {
    const state = apply(
      chatRuntimeReducer(undefined, { type: 'init' }),
      liveTurnStarted({ threadId: THREAD, requestId: 'req-1' }),
      ready('a-1'),
      turnSettled({ threadId: THREAD, requestId: 'req-1' }),
      liveTurnStarted({ threadId: THREAD, requestId: 'req-2' }),
      ready('a-1'),
      ready('a-2')
    );
    expect(owner(state, 'a-1')).toBe('req-1');
    expect(owner(state, 'a-2')).toBe('req-2');
  });

  it('keeps the owner through a failure and its retry', () => {
    const state = apply(
      chatRuntimeReducer(undefined, { type: 'init' }),
      liveTurnStarted({ threadId: THREAD, requestId: 'req-1' }),
      upsertArtifactInProgressForThread({
        threadId: THREAD,
        artifactId: 'a-1',
        kind: 'document',
        title: 'a-1',
      }),
      upsertArtifactFailedForThread({
        threadId: THREAD,
        artifactId: 'a-1',
        kind: 'document',
        title: 'a-1',
        error: 'boom',
      }),
      turnSettled({ threadId: THREAD, requestId: 'req-1' }),
      ready('a-1')
    );
    expect(owner(state, 'a-1')).toBe('req-1');
  });

  it('leaves an artifact first seen outside any turn unowned', () => {
    const state = apply(chatRuntimeReducer(undefined, { type: 'init' }), ready('a-1'));
    expect(owner(state, 'a-1')).toBeUndefined();
  });

  it('takes the turn the event names over the live turn', () => {
    const state = apply(
      chatRuntimeReducer(undefined, { type: 'init' }),
      liveTurnStarted({ threadId: THREAD, requestId: 'req-2' }),
      upsertArtifactReadyForThread({
        threadId: THREAD,
        artifactId: 'a-1',
        kind: 'document',
        title: 'a-1',
        path: '/tmp/a-1.md',
        sizeBytes: 12,
        requestId: 'req-1',
      })
    );
    expect(owner(state, 'a-1')).toBe('req-1');
  });
});
