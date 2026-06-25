import { describe, expect, it } from 'vitest';

import reducer, {
  clearAllChatRuntime,
  clearArtifactsForThread,
  clearRuntimeForThread,
  upsertArtifactFailedForThread,
  upsertArtifactInProgressForThread,
  upsertArtifactReadyForThread,
} from '../chatRuntimeSlice';

describe('chatRuntimeSlice — artifact lifecycle (#2779)', () => {
  it('upserts a new in_progress snapshot for a thread', () => {
    const next = reducer(
      undefined,
      upsertArtifactInProgressForThread({
        threadId: 't-1',
        artifactId: 'a-1',
        kind: 'presentation',
        title: 'Deck',
      })
    );

    const list = next.artifactsByThread['t-1'];
    expect(list).toHaveLength(1);
    expect(list[0]).toMatchObject({
      artifactId: 'a-1',
      kind: 'presentation',
      title: 'Deck',
      status: 'in_progress',
    });
    expect(typeof list[0].updatedAt).toBe('number');
  });

  it('promotes the in_progress entry to ready in place, preserving order', () => {
    const inProgress = reducer(
      undefined,
      upsertArtifactInProgressForThread({
        threadId: 't-1',
        artifactId: 'a-1',
        kind: 'presentation',
        title: 'Deck',
      })
    );
    const withSibling = reducer(
      inProgress,
      upsertArtifactInProgressForThread({
        threadId: 't-1',
        artifactId: 'a-2',
        kind: 'document',
        title: 'Notes',
      })
    );

    const ready = reducer(
      withSibling,
      upsertArtifactReadyForThread({
        threadId: 't-1',
        artifactId: 'a-1',
        kind: 'presentation',
        title: 'Deck',
        path: 'a-1/deck.pptx',
        sizeBytes: 4096,
      })
    );

    const list = ready.artifactsByThread['t-1'];
    expect(list).toHaveLength(2);
    // first slot stays first — replaced in place, not appended.
    expect(list[0]).toMatchObject({
      artifactId: 'a-1',
      status: 'ready',
      path: 'a-1/deck.pptx',
      sizeBytes: 4096,
    });
    expect(list[1].artifactId).toBe('a-2');
  });

  it('records a failed artifact with the producer reason', () => {
    const next = reducer(
      undefined,
      upsertArtifactFailedForThread({
        threadId: 't-1',
        artifactId: 'a-1',
        kind: 'presentation',
        title: 'Deck',
        error: 'pip install failed',
      })
    );

    expect(next.artifactsByThread['t-1'][0]).toMatchObject({
      artifactId: 'a-1',
      status: 'failed',
      error: 'pip install failed',
    });
  });

  it('appends new artifactIds in insertion order', () => {
    const a = reducer(
      undefined,
      upsertArtifactReadyForThread({
        threadId: 't-1',
        artifactId: 'a-1',
        kind: 'document',
        title: 'Doc',
        path: 'a-1/doc.pdf',
        sizeBytes: 100,
      })
    );
    const b = reducer(
      a,
      upsertArtifactReadyForThread({
        threadId: 't-1',
        artifactId: 'a-2',
        kind: 'image',
        title: 'Pic',
        path: 'a-2/pic.png',
        sizeBytes: 200,
      })
    );

    const ids = b.artifactsByThread['t-1'].map(entry => entry.artifactId);
    expect(ids).toEqual(['a-1', 'a-2']);
  });

  it('keeps thread buckets isolated', () => {
    const a = reducer(
      undefined,
      upsertArtifactReadyForThread({
        threadId: 't-1',
        artifactId: 'a-1',
        kind: 'document',
        title: 'Doc',
        path: 'a-1/doc.pdf',
        sizeBytes: 100,
      })
    );
    const b = reducer(
      a,
      upsertArtifactReadyForThread({
        threadId: 't-2',
        artifactId: 'a-2',
        kind: 'image',
        title: 'Pic',
        path: 'a-2/pic.png',
        sizeBytes: 200,
      })
    );

    expect(b.artifactsByThread['t-1']).toHaveLength(1);
    expect(b.artifactsByThread['t-2']).toHaveLength(1);
    expect(b.artifactsByThread['t-1'][0].artifactId).toBe('a-1');
    expect(b.artifactsByThread['t-2'][0].artifactId).toBe('a-2');
  });

  it('clearArtifactsForThread drops just that thread bucket', () => {
    const populated = reducer(
      undefined,
      upsertArtifactReadyForThread({
        threadId: 't-1',
        artifactId: 'a-1',
        kind: 'image',
        title: 'P',
        path: 'a-1/p.png',
        sizeBytes: 1,
      })
    );
    const cleared = reducer(populated, clearArtifactsForThread({ threadId: 't-1' }));
    expect(cleared.artifactsByThread['t-1']).toBeUndefined();
  });

  it('clearRuntimeForThread does NOT wipe artifact history', () => {
    // ArtifactCard renders inline in the message timeline so the snapshot
    // must survive turn boundaries — historic artifacts stay visible.
    const populated = reducer(
      undefined,
      upsertArtifactReadyForThread({
        threadId: 't-1',
        artifactId: 'a-1',
        kind: 'image',
        title: 'P',
        path: 'a-1/p.png',
        sizeBytes: 1,
      })
    );
    const cleared = reducer(populated, clearRuntimeForThread({ threadId: 't-1' }));
    expect(cleared.artifactsByThread['t-1']).toHaveLength(1);
  });

  it('clearAllChatRuntime drops all artifact buckets', () => {
    const populated = reducer(
      undefined,
      upsertArtifactReadyForThread({
        threadId: 't-1',
        artifactId: 'a-1',
        kind: 'image',
        title: 'P',
        path: 'a-1/p.png',
        sizeBytes: 1,
      })
    );
    const cleared = reducer(populated, clearAllChatRuntime());
    expect(cleared.artifactsByThread).toEqual({});
  });

  it('failed -> ready replays in place without duplicating', () => {
    // Useful if a producer retries: failed snapshot should be replaced
    // by ready, not stacked.
    const failed = reducer(
      undefined,
      upsertArtifactFailedForThread({
        threadId: 't-1',
        artifactId: 'a-1',
        kind: 'presentation',
        title: 'Deck',
        error: 'boom',
      })
    );
    const ready = reducer(
      failed,
      upsertArtifactReadyForThread({
        threadId: 't-1',
        artifactId: 'a-1',
        kind: 'presentation',
        title: 'Deck',
        path: 'a-1/deck.pptx',
        sizeBytes: 1024,
      })
    );

    const list = ready.artifactsByThread['t-1'];
    expect(list).toHaveLength(1);
    expect(list[0].status).toBe('ready');
    expect(list[0].error).toBeUndefined();
  });
});

describe('chatRuntimeSlice — in_progress no-downgrade guard (#3162)', () => {
  it('does NOT regress a ready artifact back to in_progress', () => {
    let state = reducer(
      undefined,
      upsertArtifactReadyForThread({
        threadId: 't-1',
        artifactId: 'a-1',
        kind: 'presentation',
        title: 'Deck',
        path: 'a-1/deck.pptx',
        sizeBytes: 4096,
      })
    );

    // A late / duplicate artifact_pending must not wipe the ready state.
    state = reducer(
      state,
      upsertArtifactInProgressForThread({
        threadId: 't-1',
        artifactId: 'a-1',
        kind: 'presentation',
        title: 'Deck',
      })
    );

    const list = state.artifactsByThread['t-1'];
    expect(list).toHaveLength(1);
    expect(list[0]).toMatchObject({ status: 'ready', sizeBytes: 4096 });
  });

  it('allows failed -> in_progress (an explicit retry re-shows the spinner)', () => {
    let state = reducer(
      undefined,
      upsertArtifactFailedForThread({
        threadId: 't-1',
        artifactId: 'a-1',
        kind: 'presentation',
        title: 'Deck',
        error: 'boom',
      })
    );

    state = reducer(
      state,
      upsertArtifactInProgressForThread({
        threadId: 't-1',
        artifactId: 'a-1',
        kind: 'presentation',
        title: 'Deck',
      })
    );

    const list = state.artifactsByThread['t-1'];
    expect(list).toHaveLength(1);
    expect(list[0].status).toBe('in_progress');
    expect(list[0].error).toBeUndefined();
  });
});
