/**
 * Welcome suggestion chips on the assistant-ui chat surface.
 *
 * The chips and the follow-up chips read the SAME runtime field
 * (`s.thread.suggestions`), and their render gates are complementary rather
 * than overlapping — welcome is `isNewChatView && composer.isEmpty`, follow-up
 * is `!isEmpty && !isRunning && length > 0`. Between them they partition every
 * state, so a suggestion list supplied unconditionally renders correctly on the
 * empty thread and then reappears as follow-up chips under every settled turn,
 * forever.
 *
 * The second test here is that bleed guard, and it is the one that matters:
 * the adapter gates the list on the runtime's message count, and if that gate
 * is ever lifted this suite is what catches it. See openhuman#6465, which also
 * records the constraint; the follow-up producer itself does not exist yet.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Provider } from 'react-redux';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { registerChatSurface } from '../../../providers/chatSurfaceHandlers';
import chatRuntimeReducer from '../../../store/chatRuntimeSlice';
import mascotReducer from '../../../store/mascotSlice';
import runModeReducer from '../../../store/runModeSlice';
import threadGoalReducer from '../../../store/threadGoalSlice';
import threadReducer from '../../../store/threadSlice';
import threadTodosReducer from '../../../store/threadTodosSlice';
import type { ThreadMessage } from '../../../types/thread';
import { AssistantUiChat } from './AssistantUiChat';

const THREAD_ID = 't-welcome';

/** The six approved starter prompts, as the user sees them (en). */
const CHIPS = [
  "What's on my calendar today?",
  'Summarize my unread email.',
  'Draft a reply to my last message.',
  "What did I say I'd follow up on this week?",
  'Connect a new integration.',
  'Build a flow that emails me a daily summary.',
];

function agentMessage(): ThreadMessage {
  return {
    id: 'm-1',
    content: 'An answer that has already landed.',
    type: 'text',
    extraMetadata: {},
    sender: 'agent',
    createdAt: '2026-01-01T00:00:00.000Z',
  };
}

function buildStore(messages: ThreadMessage[]) {
  return configureStore({
    reducer: combineReducers({
      thread: threadReducer,
      threadGoal: threadGoalReducer,
      threadTodos: threadTodosReducer,
      chatRuntime: chatRuntimeReducer,
      mascot: mascotReducer,
      // The composer's `/plan` / `/build` commands read it (`useRunMode`).
      runMode: runModeReducer,
    }),
    preloadedState: {
      thread: {
        threads: [
          {
            id: THREAD_ID,
            title: 'Welcome thread',
            chatId: null,
            isActive: false,
            messageCount: messages.length,
            lastMessageAt: '2026-01-01T00:00:00.000Z',
            createdAt: '2026-01-01T00:00:00.000Z',
            labels: [],
          },
        ],
        selectedThreadId: THREAD_ID,
        activeThreadIds: {},
        welcomeThreadId: null,
        messagesByThreadId: { [THREAD_ID]: messages },
        messages,
        isLoadingThreads: false,
        isLoadingMessages: false,
        messagesError: null,
      },
    } as never,
  });
}

function chat() {
  return (
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
  );
}

function renderChat(messages: ThreadMessage[]) {
  return render(<Provider store={buildStore(messages)}>{chat()}</Provider>);
}

/** Welcome chips currently on screen, by visible text, in render order. */
function welcomeChips(): string[] {
  return Array.from(document.querySelectorAll('.aui-thread-welcome-suggestion-display')).map(
    node => node.textContent?.trim() ?? ''
  );
}

/**
 * Every suggestion chip on screen, from BOTH surfaces.
 *
 * Asserting only on `.aui-thread-welcome-suggestion-display` is not enough and
 * was wrong in the first draft of this file: with the adapter's gate removed,
 * the leaked chips render through the *follow-up* row
 * (`.aui-thread-followup-suggestion`, different markup entirely), so a
 * welcome-only selector reports "no chips" while six of them are on screen and
 * the bleed guard passes vacuously. Query both, so the guard fails for the
 * reason it claims to.
 */
function allSuggestionChips(): string[] {
  return Array.from(
    document.querySelectorAll(
      '.aui-thread-welcome-suggestion-display, .aui-thread-followup-suggestion'
    )
  ).map(node => node.textContent?.trim() ?? '');
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('welcome suggestion chips', () => {
  it('renders the six starter prompts on an empty thread', () => {
    renderChat([]);

    // Against the rendered DOM, not the adapter's return value: the point is
    // that the chips reach the screen, not that a key exists on an object.
    expect(welcomeChips()).toEqual(CHIPS);
  });

  it('renders NO chips on either surface once the thread has a message', () => {
    renderChat([agentMessage()]);

    // Guard the absence so it cannot pass vacuously on an empty render: prove
    // the transcript actually rendered first. Without this, a crash or a blank
    // tree would satisfy the assertion below and the bleed would ship.
    expect(screen.getByText('An answer that has already landed.')).toBeTruthy();

    // BOTH surfaces. The leak shows up as follow-up chips, not welcome ones —
    // see `allSuggestionChips`. Revert-proved: with the adapter's emptiness
    // gate removed this returns all six prompts and the assertion fails.
    expect(allSuggestionChips()).toEqual([]);
  });

  it('sends the chip text as a user turn when clicked', async () => {
    const send = vi.fn().mockResolvedValue(undefined);
    const unregister = registerChatSurface(THREAD_ID, { send });
    try {
      renderChat([]);

      await userEvent.click(screen.getByText(CHIPS[0] as string));

      await waitFor(() => expect(send).toHaveBeenCalledTimes(1));
      // The chip's own text is the prompt: `ThreadSuggestion.title` falls back
      // to `prompt` upstream, so one string is both, and they cannot drift.
      expect(send).toHaveBeenCalledWith(CHIPS[0]);
    } finally {
      unregister();
    }
  });
});
