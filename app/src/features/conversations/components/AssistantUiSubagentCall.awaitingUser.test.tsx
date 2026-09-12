/**
 * A sub-agent that stops to ask the user a question must look different from
 * one that is working, and the user must be able to answer it.
 *
 * Both halves regressed in the assistant-ui migration. `onSubagentAwaitingUser`
 * reached the surface, but the row rendered through `isActiveSubagentStatus`,
 * which folds `awaiting_user` into `running`: the delegation card showed a
 * spinning "running" chip for as long as the gate stayed open, the question was
 * never carried out of the socket event at all, and the only surface that could
 * have shown it (`SubagentDrawer`) lives inside the unreachable
 * `legacyMainPanel`.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Provider } from 'react-redux';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { AssistantUiRuntimeProvider } from '../../../providers/AssistantUiRuntimeProvider';
import { __resetChatSurfaces, registerChatSurface } from '../../../providers/chatSurfaceHandlers';
import chatRuntimeReducer, {
  type SubagentActivity,
  subagentAwaitingUser,
  subagentSpawned,
} from '../../../store/chatRuntimeSlice';
import threadReducer from '../../../store/threadSlice';
import { AssistantUiSubagentCall } from './AssistantUiSubagentCall';
import { SubagentDrawerHost } from './aui/subagentDrawerHost';
import { SubagentCall } from './ChatToolParts';

vi.mock('../../../services/api/threadApi', () => ({
  threadApi: {
    getDerivedTranscript: vi
      .fn()
      .mockResolvedValue({
        threadId: 't-await',
        items: [],
        total: 0,
        hasMore: false,
        hasTranscript: false,
      }),
  },
}));

const THREAD_ID = 't-await';
const ROW_ID = `${THREAD_ID}:subagent:sub-1:researcher`;

const activity: SubagentActivity = {
  taskId: 'sub-1',
  agentId: 'researcher',
  displayName: 'Researcher',
  toolCalls: [],
};

function buildStore() {
  return configureStore({
    reducer: combineReducers({ thread: threadReducer, chatRuntime: chatRuntimeReducer }),
    preloadedState: {
      thread: {
        threads: [],
        selectedThreadId: THREAD_ID,
        activeThreadIds: {},
        welcomeThreadId: null,
        messagesByThreadId: { [THREAD_ID]: [] },
        messages: [],
        isLoadingThreads: false,
        isLoadingMessages: false,
        messagesError: null,
      },
    } as never,
  });
}

/** One `subagent_spawned`, identified the way the provider identifies them. */
function spawn(store: ReturnType<typeof buildStore>, spawnEventId?: string) {
  store.dispatch(
    subagentSpawned({
      threadId: THREAD_ID,
      round: 1,
      rowId: ROW_ID,
      taskId: 'sub-1',
      agentId: 'researcher',
      displayName: 'Researcher',
      spawnEventId,
    })
  );
}

/** Drive the row through the real reducers, exactly as the socket does. */
function parkTheDelegation(
  store: ReturnType<typeof buildStore>,
  question: string,
  spawnEventId = 'req-1:3'
) {
  spawn(store, spawnEventId);
  store.dispatch(subagentAwaitingUser({ threadId: THREAD_ID, rowId: ROW_ID, question }));
}

function rowOf(store: ReturnType<typeof buildStore>) {
  return store.getState().chatRuntime.toolTimelineByThread[THREAD_ID]?.[0];
}

afterEach(() => __resetChatSurfaces());

describe('sub-agent awaiting user', () => {
  describe('the data half', () => {
    it('carries the question out of the socket event onto the timeline row', () => {
      const store = buildStore();
      parkTheDelegation(store, 'Which of the two repos should I patch?');

      const row = store.getState().chatRuntime.toolTimelineByThread[THREAD_ID]?.[0];
      expect(row?.status).toBe('awaiting_user');
      expect(row?.subagent?.status).toBe('awaiting_user');
      // Before the fix the reducer took only {threadId, rowId} and the
      // question — the entire content of the pause — was dropped on the floor.
      expect(row?.subagent?.awaitingQuestion).toBe('Which of the two repos should I patch?');
    });

    it('unparks the row when continue_subagent republishes the spawn', () => {
      const store = buildStore();
      parkTheDelegation(store, 'Which repo?');

      // `continue_subagent` resumes a paused child by republishing
      // `subagent_spawned` for the SAME task/agent, so the row id is identical.
      // The idempotency guard used to swallow it wholesale, leaving the card
      // asking a question the user had already answered for the rest of the run.
      // The resume is a new emission, so it carries a new `(request_id, seq)`
      // -- in practice a whole new parent turn, since the user's answer is what
      // triggers it.
      spawn(store, 'req-2:0');

      const rows = store.getState().chatRuntime.toolTimelineByThread[THREAD_ID] ?? [];
      expect(rows).toHaveLength(1); // still idempotent: no duplicate row
      expect(rows[0]?.status).toBe('running');
      expect(rows[0]?.subagent?.status).toBe('running');
      expect(rows[0]?.subagent?.awaitingQuestion).toBeUndefined();
    });

    it('keeps the pending question when the ORIGINAL spawn is redelivered', () => {
      // This socket reconnects and replays freely -- 13+ times in one measured
      // session. A redelivered `subagent_spawned` arriving after
      // `subagent_awaiting_user` is shape-identical to `continue_subagent`
      // resuming the child, so the unpark used to fire on it and the question
      // vanished while the child was still blocked on the user: a spinner with
      // nothing to answer, which is the exact bug this whole change exists to
      // remove. The redelivery repeats the identity the core stamped, so it is
      // recognisable as a replay.
      const store = buildStore();
      parkTheDelegation(store, 'Which of the two repos should I patch?', 'req-1:3');

      spawn(store, 'req-1:3'); // <- the same emission, delivered twice

      const rows = store.getState().chatRuntime.toolTimelineByThread[THREAD_ID] ?? [];
      expect(rows).toHaveLength(1);
      expect(rows[0]?.status).toBe('awaiting_user');
      expect(rows[0]?.subagent?.status).toBe('awaiting_user');
      expect(rows[0]?.subagent?.awaitingQuestion).toBe('Which of the two repos should I patch?');
    });

    it('keeps the pending question when the spawn cannot be identified at all', () => {
      // An older core stamps no `seq`, so a replay inside one request collapses
      // to the same string as the original. Failing towards "this is a replay"
      // is deliberate: a stale question is visible and still settles on
      // `subagent_done`, while a silently cleared one strands the user.
      const store = buildStore();
      parkTheDelegation(store, 'Which repo?', undefined);

      spawn(store, undefined);

      expect(rowOf(store)?.status).toBe('awaiting_user');
      expect(rowOf(store)?.subagent?.awaitingQuestion).toBe('Which repo?');
    });

    it('leaves a running row alone when its spawn is redelivered', () => {
      // The replay guard must not disturb the ordinary case it also covers.
      const store = buildStore();
      spawn(store, 'req-1:3');
      spawn(store, 'req-1:3');

      const rows = store.getState().chatRuntime.toolTimelineByThread[THREAD_ID] ?? [];
      expect(rows).toHaveLength(1);
      expect(rows[0]?.status).toBe('running');
    });
  });

  describe('the render half', () => {
    it('renders a parked delegation as awaiting input, not as a running spinner', () => {
      render(
        <AssistantUiSubagentCall
          activity={{
            ...activity,
            status: 'awaiting_user',
            awaitingQuestion: 'Which of the two repos should I patch?',
          }}
          // The assistant-ui surface passes `running` from `result === undefined`,
          // which is true for a parked delegation too. The card must not believe it.
          running
        />
      );

      expect(screen.getByTestId('subagent-awaiting-chip')).toBeInTheDocument();
      expect(screen.queryByText('running')).not.toBeInTheDocument();
      expect(screen.getByTestId('subagent-awaiting-question')).toHaveTextContent(
        'Which of the two repos should I patch?'
      );
      // The question is worthless if the card stays collapsed around it.
      expect(screen.getByTestId('assistant-ui-subagent-call')).toHaveAttribute(
        'data-state',
        'open'
      );
    });

    it('still renders an ordinary running delegation as running', () => {
      render(<AssistantUiSubagentCall activity={{ ...activity, status: 'running' }} running />);
      expect(screen.getByText('running')).toBeInTheDocument();
      expect(screen.queryByTestId('subagent-awaiting-chip')).not.toBeInTheDocument();
      expect(screen.queryByTestId('subagent-awaiting-user')).not.toBeInTheDocument();
    });

    it('offers no reply box on a read-only surface', () => {
      render(
        <AssistantUiSubagentCall
          activity={{ ...activity, status: 'awaiting_user', awaitingQuestion: 'Which repo?' }}
        />
      );
      expect(screen.getByTestId('subagent-awaiting-question')).toBeInTheDocument();
      expect(screen.queryByTestId('subagent-answer-input')).not.toBeInTheDocument();
    });
  });

  describe('answering', () => {
    it('sends the answer through the thread the composer sends through', async () => {
      const send = vi.fn(async () => {});
      registerChatSurface(THREAD_ID, { send });
      const store = buildStore();

      render(
        <Provider store={store}>
          <AssistantUiRuntimeProvider>
            <SubagentCall
              type="tool-call"
              toolName="task"
              toolCallId={ROW_ID}
              args={
                {
                  subagent_type: 'researcher',
                  progress: {
                    ...activity,
                    status: 'awaiting_user',
                    awaitingQuestion: 'Which repo?',
                  },
                } as never
              }
              argsText="{}"
              result={undefined}
              status={{ type: 'running' }}
              addResult={() => {}}
              resume={() => {}}
              respondToApproval={() => {}}
            />
          </AssistantUiRuntimeProvider>
        </Provider>
      );

      expect(screen.getByTestId('subagent-awaiting-chip')).toBeInTheDocument();

      await act(async () => {
        await userEvent.type(screen.getByTestId('subagent-answer-input'), 'the second one');
      });
      await act(async () => {
        await userEvent.click(screen.getByTestId('subagent-answer-send'));
      });

      // The orchestrator is holding the [SUBAGENT_AWAITING_USER] envelope and
      // resumes the child with continue_subagent once the user answers, so the
      // answer is an ordinary user turn on the registered chat surface.
      await waitFor(() => expect(send).toHaveBeenCalledWith('the second one'));
      expect(screen.getByTestId('subagent-answer-sent')).toBeInTheDocument();
    });
  });

  describe('opening the drawer', () => {
    /** Render the inline delegation card the way the /chat transcript does. */
    function renderInlineCall(
      store: ReturnType<typeof buildStore>,
      onOpenSubagent?: (taskId: string) => void,
      canOpenSubagent?: (taskId: string) => boolean
    ) {
      return render(
        <Provider store={store}>
          <AssistantUiRuntimeProvider>
            <SubagentDrawerHost onOpenSubagent={onOpenSubagent} canOpenSubagent={canOpenSubagent}>
              <SubagentCall
                type="tool-call"
                toolName="task"
                toolCallId={ROW_ID}
                args={{ subagent_type: 'researcher', progress: activity } as never}
                argsText="{}"
                result={undefined}
                status={{ type: 'running' }}
                addResult={() => {}}
                resume={() => {}}
                respondToApproval={() => {}}
              />
            </SubagentDrawerHost>
          </AssistantUiRuntimeProvider>
        </Provider>
      );
    }

    /**
     * The card is collapsed by default and the "View full processing" button
     * lives in its content, so every assertion here has to open it first --
     * otherwise the two negative cases pass for the wrong reason.
     */
    async function expandCard() {
      await act(async () => {
        await userEvent.click(screen.getByRole('button', { name: /Delegated to Researcher/i }));
      });
    }

    it('opens the sub-agent drawer on the delegation the card is showing', async () => {
      // This is the ONLY renderer for a delegation on the assistant-ui surface,
      // and it offered no way into `SubagentDrawer`: the legacy
      // `ToolTimelineBlock` passes `onView` per row, and the one remaining
      // launcher (`BackgroundProcessesPanel`) lists async/typed spawns only, so
      // every other delegation's persisted worker conversation was unreachable.
      const store = buildStore();
      spawn(store, 'req-1:3');
      const onOpenSubagent = vi.fn();

      renderInlineCall(store, onOpenSubagent, () => true);
      await expandCard();

      await act(async () => {
        await userEvent.click(screen.getByTestId('subagent-view-processing'));
      });
      expect(onOpenSubagent).toHaveBeenCalledWith('sub-1');
    });

    it('offers nothing when no host is mounted', async () => {
      // The read-only mounts of this card (the drawer itself, past-turn
      // insights) render outside the host and must not grow a dead button.
      const store = buildStore();
      spawn(store, 'req-1:3');

      renderInlineCall(store, undefined, () => true);
      await expandCard();

      expect(screen.getByTestId('subagent-activity')).toBeInTheDocument();
      expect(screen.queryByTestId('subagent-view-processing')).not.toBeInTheDocument();
    });

    it('offers nothing for a delegation the drawer cannot resolve', async () => {
      // `TranscriptOverlays` looks the row up by `taskId` in the thread's live
      // timeline and renders nothing when it is absent, so a part replayed from
      // the settled core transcript would get a button opening an empty sheet.
      const store = buildStore();
      spawn(store, 'req-1:3');

      renderInlineCall(store, vi.fn(), () => false);
      await expandCard();

      expect(screen.getByTestId('subagent-activity')).toBeInTheDocument();
      expect(screen.queryByTestId('subagent-view-processing')).not.toBeInTheDocument();
    });
  });
});
