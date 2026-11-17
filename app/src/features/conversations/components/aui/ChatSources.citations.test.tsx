/**
 * `[n]` in the model's own answer text becomes an inline citation marker
 * (`elements/inline-citation.tsx`'s `CitationMarker`) instead of plain text,
 * when the turn has that many sources — mounted through the live `/chat`
 * surface (`AssistantUiChat`) the same way `ChatSources.test.tsx` proves its
 * own wiring, not by rendering `MarkdownText` in isolation with a hand-built
 * source list.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { threadApi } from '../../../../services/api/threadApi';
import chatRuntimeReducer from '../../../../store/chatRuntimeSlice';
import mascotReducer from '../../../../store/mascotSlice';
import runModeReducer from '../../../../store/runModeSlice';
import threadGoalReducer from '../../../../store/threadGoalSlice';
import threadReducer from '../../../../store/threadSlice';
import threadTodosReducer from '../../../../store/threadTodosSlice';
import type { DerivedDisplayItem } from '../../../../types/derivedTranscript';
import type { ThreadMessage } from '../../../../types/thread';
import { AssistantUiChat } from '../AssistantUiChat';

const THREAD_ID = 't-citations';
const REQUEST_ID = 'req-citations';

function toolCall(callId: string, url: string): DerivedDisplayItem {
  return { kind: 'toolCall', callId, name: 'web_fetch', args: { url }, status: 'success' };
}

function page(...newestFirst: DerivedDisplayItem[]) {
  return {
    items: [...newestFirst, { kind: 'turnBoundary', requestId: REQUEST_ID } as DerivedDisplayItem],
    hasTranscript: true,
    hasMore: false,
  };
}

function agentMessage(content: string): ThreadMessage {
  return {
    id: 'm-1',
    content,
    type: 'text',
    extraMetadata: { requestId: REQUEST_ID },
    sender: 'agent',
    createdAt: '2026-01-01T00:00:00.000Z',
  };
}

function buildStore(message: ThreadMessage) {
  return configureStore({
    reducer: combineReducers({
      thread: threadReducer,
      threadGoal: threadGoalReducer,
      threadTodos: threadTodosReducer,
      chatRuntime: chatRuntimeReducer,
      mascot: mascotReducer,
      runMode: runModeReducer,
    }),
    preloadedState: {
      thread: {
        threads: [
          {
            id: THREAD_ID,
            title: 'Citations thread',
            chatId: null,
            isActive: false,
            messageCount: 1,
            lastMessageAt: '2026-01-01T00:00:00.000Z',
            createdAt: '2026-01-01T00:00:00.000Z',
            labels: [],
          },
        ],
        selectedThreadId: THREAD_ID,
        activeThreadIds: {},
        welcomeThreadId: null,
        messagesByThreadId: { [THREAD_ID]: [message] },
        messages: [message],
        isLoadingThreads: false,
        isLoadingMessages: false,
        messagesError: null,
      },
    } as never,
  });
}

function renderChat(message: ThreadMessage) {
  return render(
    <Provider store={buildStore(message)}>
      <AssistantUiChat
        model={null}
        onModelChange={vi.fn()}
        inputValue=""
        onInputValueChange={vi.fn()}
        attachments={[]}
        onAttachFiles={vi.fn()}
        onRemoveAttachment={vi.fn()}
        maxAttachments={5}
        attachmentsEnabled={false}
        attachmentInteractionBlocked={false}
        onAttachmentOnlySend={vi.fn()}
      />
    </Provider>
  );
}

beforeEach(() => {
  vi.restoreAllMocks();
});

describe('inline citation markers', () => {
  it('turns [1] into a citation marker when the turn has a matching source', async () => {
    vi.spyOn(threadApi, 'getDerivedTranscript').mockResolvedValue(
      page(toolCall('c1', 'https://example.com/a')) as never
    );

    renderChat(agentMessage('That page says so [1].'));

    await waitFor(() => expect(screen.getByTestId('turn-sources')).toBeTruthy());
    const marker = screen.getByRole('button', { name: '1' });
    expect(marker).toBeTruthy();
    // Plain text, not a markdown link: the bracketed number itself is gone
    // from the rendered prose, replaced by the marker.
    expect(screen.queryByText('[1]')).toBeNull();
  });

  it('leaves [1] as plain text when the turn has no sources', async () => {
    vi.spyOn(threadApi, 'getDerivedTranscript').mockResolvedValue(page() as never);

    renderChat(agentMessage('See item [1] on the list.'));

    await waitFor(() => expect(screen.getByText(/See item/)).toBeTruthy());
    expect(screen.queryByRole('button', { name: '1' })).toBeNull();
  });
});
