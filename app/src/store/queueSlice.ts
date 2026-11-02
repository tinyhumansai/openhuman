/**
 * The core's run queue, per thread, as the composer's message queue renders it.
 *
 * Two lists with two owners:
 *
 * - `itemsByThread` is the core's: messages sitting in a running turn's queue,
 *   filled and drained by the `queue_item_queued` / `queue_item_delivered` /
 *   `queue_item_removed` socket events. It is what the user sees, so it can
 *   never show a message the core no longer holds.
 * - `pendingFollowupsByThread` keeps the local display until that queued
 *   request's terminal event confirms its user row was persisted by the core.
 *   Queue events carry only a preview; the ack's request ID links them to the
 *   eventual persisted row without a second frontend append.
 *
 * The two are linked by preview text only. The core mints the item id and the
 * `channel_web_chat` ack does not return it, so removing an item drops the
 * pending follow-up whose preview matches, keeping a cancelled message out of
 * the transcript.
 */
import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

import { parseAttachmentReferences } from '../lib/attachments';
import type { QueueItemPayload } from '../services/chatService';
import type { ContentPart } from '../services/chatWire';
import type { ThreadMessage } from '../types/thread';
import { clearAllChatRuntime, clearRuntimeForThread, endInferenceTurn } from './chatRuntimeSlice';
import { resetUserScopedState } from './resetActions';

/** A message waiting in a running turn's core queue. */
export interface RunQueueItem {
  /** The core's queue item id (`QueueItemPayload.id`). */
  id: string;
  /** `steer` / `followup` / `collect` when the core names it. */
  lane: string | null;
  /** The core's clipped preview of the message text. */
  textPreview: string;
}

/** A follow-up this client queued, held until it can be persisted. */
export interface PendingFollowup {
  /** The user message shown while the core is executing this queued request. */
  message: ThreadMessage;
  /** Sanitized opaque refs returned by the queue ack. */
  attachments: ContentPart[];
  /** The same ID used by the queue item and the turn's terminal event. */
  requestId: string;
  /** `clipQueuePreview` of the text sent to the core; matches the item's preview. */
  preview: string;
}

export interface QueueState {
  itemsByThread: Record<string, RunQueueItem[]>;
  pendingFollowupsByThread: Record<string, PendingFollowup[]>;
}

const initialState: QueueState = { itemsByThread: {}, pendingFollowupsByThread: {} };

/** Mirrors core queue previews: caption or filenames, clipped at 80 code points. */
const QUEUE_PREVIEW_CHARS = 80;

export function clipQueuePreview(text: string, attachments: ContentPart[] = []): string {
  const names: string[] = attachments.map(part =>
    'media' in part ? (part.media.fileName ?? 'attachment') : 'attachment'
  );
  const caption = text
    .replace(/\[(IMAGE|FILE|ATTACHMENT):([^\]]+)\]/g, (marker, kind: string, source: string) => {
      if (kind === 'ATTACHMENT') {
        const parsed = parseAttachmentReferences(marker);
        if (parsed.attachments.length === 0) return marker;
        names.push(parsed.attachments[0].name);
      } else {
        // Inspect only the transport header, never decode the media payload.
        const header = source.startsWith('data:') ? source.split(',', 1)[0] : '';
        const encoded = header
          .split(';')
          .find(param => param.startsWith('name='))
          ?.slice(5);
        const name =
          encoded === undefined ? '' : new URLSearchParams(`name=${encoded}`).get('name');
        names.push(name || 'attachment');
      }
      return '';
    })
    .replace(/ {2,}/g, ' ')
    .trim();
  const preview = caption || names.join(', ');
  const chars = Array.from(preview);
  if (chars.length <= QUEUE_PREVIEW_CHARS) return preview;
  return `${chars.slice(0, QUEUE_PREVIEW_CHARS).join('')}…`;
}

function dropItem(state: QueueState, threadId: string, itemId: string): RunQueueItem | null {
  const bucket = state.itemsByThread[threadId];
  const index = bucket?.findIndex(item => item.id === itemId) ?? -1;
  if (!bucket || index === -1) return null;
  const [removed] = bucket.splice(index, 1);
  if (bucket.length === 0) delete state.itemsByThread[threadId];
  return removed;
}

function clearThread(state: QueueState, threadId: string) {
  delete state.itemsByThread[threadId];
  delete state.pendingFollowupsByThread[threadId];
}

const queueSlice = createSlice({
  name: 'queue',
  initialState,
  reducers: {
    queueItemQueued: (
      state,
      action: PayloadAction<{ threadId: string; item: QueueItemPayload }>
    ) => {
      const { threadId, item } = action.payload;
      const bucket = state.itemsByThread[threadId] ?? [];
      if (bucket.some(existing => existing.id === item.id)) return;
      bucket.push({ id: item.id, lane: item.lane ?? null, textPreview: item.text_preview ?? '' });
      state.itemsByThread[threadId] = bucket;
    },
    /** The core handed the item to a turn; its message persists on turn end. */
    queueItemDelivered: (state, action: PayloadAction<{ threadId: string; itemId: string }>) => {
      dropItem(state, action.payload.threadId, action.payload.itemId);
    },
    /** The runner persisted this queued request and its terminal event arrived. */
    pendingFollowupCompleted: (
      state,
      action: PayloadAction<{ threadId: string; requestId: string }>
    ) => {
      const { threadId, requestId } = action.payload;
      const pending = state.pendingFollowupsByThread[threadId];
      if (!pending) return;
      const remaining = pending.filter(item => item.requestId !== requestId);
      if (remaining.length) state.pendingFollowupsByThread[threadId] = remaining;
      else delete state.pendingFollowupsByThread[threadId];
    },
    /** The item was taken out of the queue and will never be sent. */
    queueItemRemoved: (state, action: PayloadAction<{ threadId: string; itemId: string }>) => {
      const { threadId, itemId } = action.payload;
      const removed = dropItem(state, threadId, itemId);
      const pending = state.pendingFollowupsByThread[threadId];
      if (!removed || !pending) return;
      const index = pending.findIndex(entry => entry.preview === removed.textPreview);
      if (index === -1) return;
      pending.splice(index, 1);
      if (pending.length === 0) delete state.pendingFollowupsByThread[threadId];
    },
    /** Record a follow-up the core accepted; `text` is what was sent to it. */
    pendingFollowupAdded: (
      state,
      action: PayloadAction<{
        threadId: string;
        message: ThreadMessage;
        text: string;
        requestId?: string;
        attachments?: ContentPart[];
      }>
    ) => {
      const { threadId, message, text, attachments = [] } = action.payload;
      const requestId = action.payload.requestId ?? message.id;
      const bucket = state.pendingFollowupsByThread[threadId] ?? [];
      bucket.push({
        message,
        attachments,
        requestId,
        preview: clipQueuePreview(text, attachments),
      });
      state.pendingFollowupsByThread[threadId] = bucket;
    },
  },
  extraReducers: builder => {
    // The turn ended, so the core is dispatching whatever it still queued, and
    // `ChatRuntimeProvider` has already persisted the pending follow-ups.
    builder.addCase(endInferenceTurn, (state, action) => {
      delete state.itemsByThread[action.payload.threadId];
    });
    builder.addCase(clearRuntimeForThread, (state, action) =>
      clearThread(state, action.payload.threadId)
    );
    builder.addCase(clearAllChatRuntime, () => initialState);
    builder.addCase(resetUserScopedState, () => initialState);
  },
});

export const {
  queueItemQueued,
  queueItemDelivered,
  queueItemRemoved,
  pendingFollowupAdded,
  pendingFollowupCompleted,
} = queueSlice.actions;

export default queueSlice.reducer;
