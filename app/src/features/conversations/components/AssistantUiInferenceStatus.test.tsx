/**
 * The progress line on the assistant-ui chat surface.
 *
 * `/chat` renders `AssistantUiChat`, never `ChatThreadView`, so before the
 * `RunningStatus` slot the whole of `chatRuntime.inferenceStatusByThread` —
 * reasoning round, active tool, delegated sub-agent — was dispatched by
 * `onInferenceStart` / `onIterationStart` / `onToolCall` and rendered nowhere.
 * assistant-ui knows only `thread.isRunning`, so a long turn was a spinner with
 * no round counter and no tool name.
 *
 * These tests mount the real surface (`AssistantUiChat` → its own
 * `AssistantUiRuntimeProvider` → `Thread`) over a real store, so they cover
 * both halves of the fix: the adapter publishing the status on the runtime's
 * `extras`, and the slot rendering it.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it, vi } from 'vitest';

import chatRuntimeReducer, {
  beginInferenceTurn,
  endInferenceTurn,
  markInferenceTurnStreaming,
  setInferenceStatusForThread,
  setToolTimelineForThread,
  subagentAwaitingUser,
} from '../../../store/chatRuntimeSlice';
import mascotReducer from '../../../store/mascotSlice';
import threadReducer from '../../../store/threadSlice';
import { AssistantUiChat } from './AssistantUiChat';
import type { ThreadGoalController } from './ThreadGoalChip';

const THREAD_ID = 't-status';

function buildStore() {
  return configureStore({
    reducer: combineReducers({
      thread: threadReducer,
      chatRuntime: chatRuntimeReducer,
      mascot: mascotReducer,
    }),
    preloadedState: {
      thread: {
        threads: [
          {
            id: THREAD_ID,
            title: 'Status thread',
            chatId: null,
            isActive: true,
            messageCount: 1,
            lastMessageAt: '2026-01-01T00:00:00.000Z',
            createdAt: '2026-01-01T00:00:00.000Z',
            labels: ['general'],
          },
        ],
        selectedThreadId: THREAD_ID,
        activeThreadIds: {},
        welcomeThreadId: null,
        messagesByThreadId: {
          [THREAD_ID]: [
            {
              id: 'm-0',
              sender: 'user',
              type: 'text',
              content: 'run the suite',
              extraMetadata: {},
              createdAt: '2026-01-01T00:00:00.000Z',
            },
          ],
        },
        messages: [],
        isLoadingThreads: false,
        isLoadingMessages: false,
        messagesError: null,
      },
    } as never,
  });
}

const threadGoal: ThreadGoalController = {
  threadId: THREAD_ID,
  goal: null,
  expanded: false,
  draft: '',
  busy: false,
  setDraft: vi.fn(),
  open: vi.fn(),
  close: vi.fn(),
  toggle: vi.fn(),
  save: vi.fn(),
  complete: vi.fn(),
  pause: vi.fn(),
  resume: vi.fn(),
  clear: vi.fn(),
};

function renderChat(store: ReturnType<typeof buildStore>) {
  return render(
    <Provider store={store}>
      <AssistantUiChat
        threadGoal={threadGoal}
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

/** Put the thread in the state a live turn leaves behind. */
function startTurn(store: ReturnType<typeof buildStore>) {
  act(() => {
    store.dispatch(beginInferenceTurn({ threadId: THREAD_ID }));
    store.dispatch(markInferenceTurnStreaming({ threadId: THREAD_ID }));
  });
}

describe('inference status on the assistant-ui chat surface', () => {
  it('shows the reasoning round while the model is thinking', async () => {
    const store = buildStore();
    renderChat(store);
    startTurn(store);

    // Nothing to say before the first `iteration_start`.
    expect(screen.queryByTestId('inference-status-line')).not.toBeInTheDocument();

    act(() => {
      store.dispatch(
        setInferenceStatusForThread({
          threadId: THREAD_ID,
          status: { phase: 'thinking', iteration: 3, maxIterations: 8 },
        })
      );
    });

    expect(await screen.findByTestId('inference-status-line')).toHaveTextContent('Thinking... (3)');
  });

  it('names the running tool when no timeline row carries it', async () => {
    const store = buildStore();
    renderChat(store);
    startTurn(store);

    // `tool_use` with an empty timeline: a restored snapshot, or a row that
    // settled ahead of the status. `status.activeTool` is then the only name
    // the surface has for the work in flight.
    act(() => {
      store.dispatch(
        setInferenceStatusForThread({
          threadId: THREAD_ID,
          status: { phase: 'tool_use', iteration: 2, maxIterations: 8, activeTool: 'shell' },
        })
      );
    });

    expect(await screen.findByTestId('inference-status-line')).toHaveTextContent('Running command');
  });

  it('yields to the tool part once the running row is on screen', async () => {
    const store = buildStore();
    renderChat(store);
    startTurn(store);

    act(() => {
      store.dispatch(
        setInferenceStatusForThread({
          threadId: THREAD_ID,
          status: { phase: 'thinking', iteration: 2, maxIterations: 8 },
        })
      );
    });
    expect(await screen.findByTestId('inference-status-line')).toHaveTextContent('Thinking... (2)');

    // The tool call lands: the row is projected as a tool part that already
    // names the command, so the line must not caption it a second time.
    act(() => {
      store.dispatch(
        setToolTimelineForThread({
          threadId: THREAD_ID,
          entries: [
            { id: 'tl-1', name: 'shell', round: 2, seq: 0, status: 'running', detail: 'npm test' },
          ],
        })
      );
      store.dispatch(
        setInferenceStatusForThread({
          threadId: THREAD_ID,
          status: { phase: 'tool_use', iteration: 2, maxIterations: 8, activeTool: 'shell' },
        })
      );
    });

    expect(screen.queryByTestId('inference-status-line')).not.toBeInTheDocument();
  });

  it('keeps the paused sub-agent as the active row when the child asks the user', async () => {
    // Regression (CodeRabbit, PR #6036): `activeSubagentEntry` matched only
    // `status === 'running'`, but `subagentAwaitingUser` sets `awaiting_user` on
    // the row's TOP-LEVEL status. The lookup therefore lost the row the instant
    // the child parked on `ask_user_clarification`, and the generic status line
    // reappeared over the delegation card — announcing that the agent was
    // working at the one moment it was blocked on the user.
    const store = buildStore();
    renderChat(store);
    startTurn(store);

    const rowId = `${THREAD_ID}:subagent:task-1:researcher`;
    act(() => {
      store.dispatch(
        setToolTimelineForThread({
          threadId: THREAD_ID,
          entries: [
            {
              id: rowId,
              name: 'subagent:researcher',
              round: 1,
              seq: 0,
              status: 'running',
              subagent: {
                taskId: 'task-1',
                agentId: 'researcher',
                status: 'running',
                toolCalls: [],
              },
            },
          ],
        })
      );
      store.dispatch(
        setInferenceStatusForThread({
          threadId: THREAD_ID,
          status: {
            phase: 'subagent',
            iteration: 1,
            maxIterations: 8,
            activeSubagent: 'researcher',
          },
        })
      );
    });

    // Guards the instrument: while the child runs, the delegation card owns the
    // display and the generic line is already suppressed.
    await screen.findByText(/researcher/i);
    expect(screen.queryByTestId('inference-status-line')).not.toBeInTheDocument();

    // The child parks on the user. Driven through the real reducer, so the test
    // pins the reducer -> adapter -> render chain rather than a hand-written
    // status value.
    act(() => {
      store.dispatch(
        subagentAwaitingUser({
          threadId: THREAD_ID,
          rowId,
          question: 'Which repository should I review?',
        })
      );
    });

    // The row is still the active sub-agent, so the card keeps the floor and
    // the generic line stays away.
    expect(await screen.findByText('Which repository should I review?')).toBeInTheDocument();
    expect(screen.queryByTestId('inference-status-line')).not.toBeInTheDocument();
  });

  it('drops the line when the turn ends', async () => {
    const store = buildStore();
    renderChat(store);
    startTurn(store);

    act(() => {
      store.dispatch(
        setInferenceStatusForThread({
          threadId: THREAD_ID,
          status: { phase: 'thinking', iteration: 1, maxIterations: 8 },
        })
      );
    });
    expect(await screen.findByTestId('inference-status-line')).toHaveTextContent('Thinking... (1)');

    act(() => {
      store.dispatch(endInferenceTurn({ threadId: THREAD_ID }));
    });

    // `thread.isRunning` is false now, so the slot is gone even though the
    // status slice has not been cleared yet.
    expect(screen.queryByTestId('inference-status-line')).not.toBeInTheDocument();
  });
});
