/**
 * The quote has to survive the send, or the composer's quote chip is scenery.
 *
 * `SelectionToolbarPrimitive.Quote` sets the quote as STRUCTURE —
 * `metadata.custom.quote`, a `{ text, messageId }` — but `surface.send` takes a
 * string. Anything the adapter does not fold into that string never reaches the
 * model, and the user would watch themselves quote a paragraph while the agent
 * answered as though they had not. That is the failure this file exists for.
 */
import type { AppendMessage } from '@assistant-ui/react';
import { configureStore } from '@reduxjs/toolkit';
import { renderHook } from '@testing-library/react';
import type { ReactNode } from 'react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { editMessage } from '../../services/chatService';
import type { ContentPart } from '../../services/chatWire';
import chatRuntimeReducer from '../../store/chatRuntimeSlice';
import threadReducer, { loadThreadMessages } from '../../store/threadSlice';
import { registerChatSurface } from '../chatSurfaceHandlers';
import { useOpenHumanExternalStore } from '../useOpenHumanExternalStore';

vi.mock('../../services/api/threadApi', () => ({
  threadApi: {
    getDerivedTranscript: vi
      .fn()
      .mockResolvedValue({
        threadId: 't-quote',
        items: [],
        total: 0,
        hasMore: false,
        hasTranscript: false,
      }),
  },
}));
vi.mock('../../services/chatService', () => ({
  editMessage: vi.fn().mockResolvedValue(undefined),
  regenerateMessage: vi.fn().mockResolvedValue(undefined),
}));

const THREAD_ID = 't-quote';

function appended(over: Partial<AppendMessage> = {}): AppendMessage {
  return {
    role: 'user',
    content: [{ type: 'text', text: 'what does this mean?' }],
    parentId: null,
    sourceId: null,
    runConfig: undefined,
    attachments: [],
    metadata: { custom: {} },
    createdAt: new Date(),
    ...over,
  } as unknown as AppendMessage;
}

function withQuote(text: string): AppendMessage {
  return appended({
    metadata: { custom: { quote: { text, messageId: 'm-1' } } },
  } as unknown as Partial<AppendMessage>);
}

function mount(
  store = configureStore({ reducer: { thread: threadReducer, chatRuntime: chatRuntimeReducer } })
) {
  const wrapper = ({ children }: { children: ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );
  return renderHook(() => useOpenHumanExternalStore(THREAD_ID), { wrapper });
}

describe('a quoted excerpt reaching the core', () => {
  let sent: string[];

  beforeEach(() => {
    sent = [];
    registerChatSurface(THREAD_ID, {
      send: async (text: string) => {
        sent.push(text);
      },
    });
  });

  it('sends the excerpt as a blockquote above the question', async () => {
    const { result } = mount();
    await result.current.onNew(withQuote('The mitochondria is the powerhouse of the cell.'));

    expect(sent).toEqual([
      '> The mitochondria is the powerhouse of the cell.\n\nwhat does this mean?',
    ]);
  });

  it('prefixes every line of a multi-line excerpt', async () => {
    // A single `>` on the first line only is not a blockquote — markdown ends
    // it at the first unprefixed line, so the rest of the excerpt would read as
    // the user's own words.
    const { result } = mount();
    await result.current.onNew(withQuote('first line\nsecond line'));

    expect(sent).toEqual(['> first line\n> second line\n\nwhat does this mean?']);
  });

  it('sends the question unchanged when nothing is quoted', async () => {
    const { result } = mount();
    await result.current.onNew(appended());

    expect(sent).toEqual(['what does this mean?']);
  });

  it('ignores a whitespace-only quote rather than sending an empty blockquote', async () => {
    const { result } = mount();
    await result.current.onNew(withQuote('   \n  '));

    expect(sent).toEqual(['what does this mean?']);
  });

  it('keeps the source row opaque attachments when editing its text', async () => {
    const store = configureStore({
      reducer: { thread: threadReducer, chatRuntime: chatRuntimeReducer },
    });
    const attachment: ContentPart = {
      kind: 'image' as const,
      media: {
        opaqueId: 'stored-image-1',
        contentType: 'image/png',
        codec: null,
        durationMs: null,
        fileName: null,
        height: null,
        kind: 'image',
        localCachePath: null,
        path: null,
        platformMessageId: null,
        sizeBytes: null,
        transcribed: false,
        url: null,
        width: null,
      },
      caption: null,
      altText: null,
    };
    store.dispatch(
      loadThreadMessages.fulfilled(
        {
          threadId: THREAD_ID,
          messages: [
            {
              id: 'user-image-row',
              content: 'describe this',
              type: 'text',
              extraMetadata: {},
              sender: 'user',
              createdAt: new Date().toISOString(),
              attachments: [attachment],
            },
          ],
        },
        'load-image-row',
        THREAD_ID
      )
    );
    const { result } = mount(store);

    await result.current.onEdit(
      appended({
        sourceId: 'user-image-row',
        content: [{ type: 'text', text: 'describe this more carefully' }],
      })
    );

    expect(editMessage).toHaveBeenCalledWith({
      threadId: THREAD_ID,
      messageId: 'user-image-row',
      content: 'describe this more carefully',
      attachments: [attachment],
    });
  });
});
