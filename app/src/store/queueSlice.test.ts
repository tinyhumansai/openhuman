import { describe, expect, it } from 'vitest';

import type { ThreadMessage } from '../types/thread';
import {
  beginInferenceTurn,
  clearAllChatRuntime,
  clearRuntimeForThread,
  endInferenceTurn,
} from './chatRuntimeSlice';
import reducer, {
  clipQueuePreview,
  pendingFollowupAdded,
  pendingFollowupCompleted,
  queueItemDelivered,
  queueItemQueued,
  queueItemRemoved,
} from './queueSlice';

const message = (id: string, content: string): ThreadMessage => ({
  id,
  content,
  type: 'text',
  extraMetadata: {},
  sender: 'user',
  createdAt: '2026-01-01T00:00:00.000Z',
});

const queued = (threadId: string, id: string, text: string) =>
  queueItemQueued({ threadId, item: { id, text_preview: text } });

describe('queueSlice — core run-queue items', () => {
  it('appends queued items in order per thread', () => {
    let state = reducer(undefined, queued('t1', 'q1', 'first'));
    state = reducer(state, queued('t1', 'q2', 'second'));
    state = reducer(state, queued('t2', 'q3', 'other'));

    expect(state.itemsByThread.t1).toEqual([
      { id: 'q1', lane: null, textPreview: 'first' },
      { id: 'q2', lane: null, textPreview: 'second' },
    ]);
    expect(state.itemsByThread.t2).toHaveLength(1);
  });

  it('ignores a duplicate queued event for the same item id', () => {
    let state = reducer(undefined, queued('t1', 'q1', 'first'));
    state = reducer(state, queued('t1', 'q1', 'first'));

    expect(state.itemsByThread.t1).toHaveLength(1);
  });

  it('keeps the lane and falls back to an empty preview when the core omits one', () => {
    const state = reducer(
      undefined,
      queueItemQueued({ threadId: 't1', item: { id: 'q1', lane: 'steer' } })
    );

    expect(state.itemsByThread.t1).toEqual([{ id: 'q1', lane: 'steer', textPreview: '' }]);
  });

  it('drops a delivered item and prunes the empty bucket', () => {
    let state = reducer(undefined, queued('t1', 'q1', 'first'));
    state = reducer(state, queued('t1', 'q2', 'second'));

    state = reducer(state, queueItemDelivered({ threadId: 't1', itemId: 'q1' }));
    expect(state.itemsByThread.t1.map(i => i.id)).toEqual(['q2']);

    state = reducer(state, queueItemDelivered({ threadId: 't1', itemId: 'q2' }));
    expect(state.itemsByThread.t1).toBeUndefined();
  });

  it('keeps pending follow-ups after the core delivers their queue item', () => {
    let state = reducer(
      undefined,
      pendingFollowupAdded({ threadId: 't1', message: message('m1', 'hi'), text: 'hi' })
    );
    state = reducer(state, queued('t1', 'q1', 'hi'));
    state = reducer(state, queueItemDelivered({ threadId: 't1', itemId: 'q1' }));

    expect(state.pendingFollowupsByThread.t1.map(p => p.message.id)).toEqual(['m1']);
  });
});

describe('queueSlice — pending follow-up persistence', () => {
  it('records follow-ups in send order with a core-shaped preview', () => {
    let state = reducer(
      undefined,
      pendingFollowupAdded({ threadId: 't1', message: message('m1', 'one'), text: 'one' })
    );
    state = reducer(
      state,
      pendingFollowupAdded({ threadId: 't1', message: message('m2', 'two'), text: 'two' })
    );

    expect(state.pendingFollowupsByThread.t1.map(p => [p.message.id, p.preview])).toEqual([
      ['m1', 'one'],
      ['m2', 'two'],
    ]);
  });

  it('removing an item also drops the follow-up whose preview it carries', () => {
    let state = reducer(
      undefined,
      pendingFollowupAdded({ threadId: 't1', message: message('m1', 'keep'), text: 'keep' })
    );
    state = reducer(
      state,
      pendingFollowupAdded({ threadId: 't1', message: message('m2', 'drop'), text: 'drop' })
    );
    state = reducer(state, queued('t1', 'q1', 'keep'));
    state = reducer(state, queued('t1', 'q2', 'drop'));

    state = reducer(state, queueItemRemoved({ threadId: 't1', itemId: 'q2' }));

    expect(state.itemsByThread.t1.map(i => i.id)).toEqual(['q1']);
    expect(state.pendingFollowupsByThread.t1.map(p => p.message.id)).toEqual(['m1']);
  });

  it('a removal for an unknown item leaves pending follow-ups alone', () => {
    let state = reducer(
      undefined,
      pendingFollowupAdded({ threadId: 't1', message: message('m1', 'x'), text: 'x' })
    );
    state = reducer(state, queueItemRemoved({ threadId: 't1', itemId: 'nope' }));

    expect(state.pendingFollowupsByThread.t1).toHaveLength(1);
  });

  it('prunes the pending bucket once its last follow-up is removed', () => {
    let state = reducer(
      undefined,
      pendingFollowupAdded({ threadId: 't1', message: message('m1', 'x'), text: 'x' })
    );
    state = reducer(state, queued('t1', 'q1', 'x'));
    state = reducer(state, queueItemRemoved({ threadId: 't1', itemId: 'q1' }));

    expect(state.pendingFollowupsByThread.t1).toBeUndefined();
    expect(state.itemsByThread.t1).toBeUndefined();
  });
});

describe('queueSlice — chat runtime lifecycle', () => {
  const seeded = () => {
    let state = reducer(undefined, queued('t1', 'q1', 'a'));
    state = reducer(state, queued('t2', 'q2', 'b'));
    state = reducer(
      state,
      pendingFollowupAdded({ threadId: 't1', message: message('m1', 'a'), text: 'a' })
    );
    return reducer(
      state,
      pendingFollowupAdded({ threadId: 't2', message: message('m2', 'b'), text: 'b' })
    );
  };

  it('keeps pending follow-ups until their own persisted terminal event', () => {
    let state = reducer(seeded(), beginInferenceTurn({ threadId: 't1' }));
    state = reducer(state, endInferenceTurn({ threadId: 't1' }));

    expect(state.itemsByThread.t1).toBeUndefined();
    expect(state.pendingFollowupsByThread.t1).toHaveLength(1);
    expect(state.itemsByThread.t2).toBeDefined();
    state = reducer(state, pendingFollowupCompleted({ threadId: 't1', requestId: 'm1' }));
    expect(state.pendingFollowupsByThread.t1).toBeUndefined();
  });

  it('clearRuntimeForThread clears one thread, clearAllChatRuntime clears all', () => {
    const perThread = reducer(seeded(), clearRuntimeForThread({ threadId: 't1' }));
    expect(perThread.itemsByThread.t1).toBeUndefined();
    expect(perThread.pendingFollowupsByThread.t1).toBeUndefined();
    expect(perThread.pendingFollowupsByThread.t2).toBeDefined();

    const all = reducer(seeded(), clearAllChatRuntime());
    expect(all).toEqual({ itemsByThread: {}, pendingFollowupsByThread: {} });
  });
});

describe('clipQueuePreview', () => {
  it('matches raw and staged file-only previews using decoded original filenames', () => {
    const raw = '[FILE:data:application/zip;name=Archive%20%F0%9F%98%80.zip;base64,AQID]';
    const staged = `[ATTACHMENT:${encodeURIComponent(JSON.stringify({ path: 'uploads/t/id/Archive.zip', name: 'Archive 😀.zip', mime: 'application/zip', size_bytes: 3 }))}]`;
    expect(clipQueuePreview(raw)).toBe('Archive 😀.zip');
    expect(clipQueuePreview(staged)).toBe(clipQueuePreview(raw));
    expect(clipQueuePreview('[IMAGE:data:image/png;base64,AQID]')).toBe('attachment');
  });

  it('keeps short captions stable across staging and preserves intentional paragraph breaks', () => {
    const raw = '[IMAGE:data:image/png;name=photo.png;base64,AQID]';
    const staged = `[ATTACHMENT:${encodeURIComponent(JSON.stringify({ path: 'uploads/t/id/photo.png', name: 'photo.png', mime: 'image/png', size_bytes: 3 }))}]`;
    expect(clipQueuePreview(`Look  here ${raw}`)).toBe('Look here');
    expect(clipQueuePreview(`Look  here ${staged}`)).toBe('Look here');
    expect(clipQueuePreview(`first\n\nsecond ${raw}`)).toBe('first\n\nsecond');
  });

  it('joins file-only names in source order without decoding payload bytes', () => {
    const staged = `[ATTACHMENT:${encodeURIComponent(JSON.stringify({ path: 'uploads/t/id/a.zip', name: 'a.zip', mime: 'application/zip', size_bytes: 3 }))}]`;
    expect(clipQueuePreview(`${staged} [IMAGE:data:image/png;name=b.png;base64,not-base64]`)).toBe(
      'a.zip, b.png'
    );
  });

  it('removes a pending raw upload when the core cancels its staged queue item', () => {
    const raw = '[FILE:data:application/zip;name=archive.zip;base64,AQID]';
    const staged = `[ATTACHMENT:${encodeURIComponent(JSON.stringify({ path: 'uploads/t/id/archive.zip', name: 'archive.zip', mime: 'application/zip', size_bytes: 3 }))}]`;
    let state = reducer(
      undefined,
      pendingFollowupAdded({ threadId: 't-upload', message: message('m-upload', raw), text: raw })
    );
    state = reducer(state, queued('t-upload', 'q-upload', clipQueuePreview(staged)));
    state = reducer(state, queueItemRemoved({ threadId: 't-upload', itemId: 'q-upload' }));
    expect(state.pendingFollowupsByThread['t-upload']).toBeUndefined();
    expect(state.itemsByThread['t-upload']).toBeUndefined();
  });

  it('matches the core clip: 80 code points, then an ellipsis', () => {
    expect(clipQueuePreview('short')).toBe('short');
    expect(clipQueuePreview('x'.repeat(80))).toBe('x'.repeat(80));
    expect(clipQueuePreview('x'.repeat(81))).toBe(`${'x'.repeat(80)}…`);
    // Astral characters count once, as Rust's `chars()` does.
    expect(clipQueuePreview('😀'.repeat(81))).toBe(`${'😀'.repeat(80)}…`);
  });
});

describe('clipQueuePreview with typed attachments', () => {
  const file = (name?: string) => ({
    kind: 'file' as const,
    fileName: name ?? null,
    caption: null,
    media: {
      kind: 'document' as const,
      url: 'data:application/pdf;base64,AAA',
      fileName: name ?? null,
      contentType: null,
      sizeBytes: null,
      codec: null,
      durationMs: null,
      height: null,
      localCachePath: null,
      path: null,
      platformMessageId: null,
      transcribed: false,
      width: null,
    },
  });

  it('uses the caption when there is one', () => {
    expect(clipQueuePreview('see this', [file('a.pdf')])).toBe('see this');
  });

  it('falls back to the attachment file names when there is no caption', () => {
    expect(clipQueuePreview('', [file('a.pdf'), file('b.pdf')])).toBe('a.pdf, b.pdf');
    expect(clipQueuePreview('', [file()])).toBe('attachment');
  });
});
