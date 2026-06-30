import { render, waitFor } from '@testing-library/react';
import { act } from 'react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import * as chatService from '../../services/chatService';
import { threadApi } from '../../services/api/threadApi';
import { store } from '../../store';
import {
  clearAllChatRuntime,
  enqueueFollowup,
  registerParallelRequest,
  resetSessionTokenUsage,
  setPendingPlanReviewForThread,
} from '../../store/chatRuntimeSlice';
import { setStatusForUser } from '../../store/socketSlice';
import { clearAllThreads, loadThreads, setSelectedThread } from '../../store/threadSlice';
import ChatRuntimeProvider, { findPendingDelegationContext } from '../ChatRuntimeProvider';

vi.mock('../../services/chatService', async () => {
  const actual = await vi.importActual<typeof chatService>('../../services/chatService');
  return { ...actual, subscribeChatEvents: vi.fn() };
});

vi.mock('../../services/api/threadApi', () => ({
  threadApi: {
    createNewThread: vi.fn(),
    getThreads: vi.fn(),
    getThreadMessages: vi.fn(),
    appendMessage: vi.fn(),
    generateTitleIfNeeded: vi.fn(),
    updateMessage: vi.fn(),
    deleteThread: vi.fn(),
    purge: vi.fn(),
    getTaskBoard: vi.fn(),
    putTaskBoard: vi.fn(),
  },
}));

vi.mock('../../hooks/usageRefresh', () => ({ requestUsageRefresh: vi.fn() }));

const mockRefetchSnapshot = vi.fn();
vi.mock('../../hooks/useRefetchSnapshotOnTurnEnd', () => ({
  useRefetchSnapshotOnTurnEnd: () => ({ refetch: mockRefetchSnapshot }),
}));

function renderProvider(): chatService.ChatEventListeners {
  let captured: chatService.ChatEventListeners = {};
  vi.mocked(chatService.subscribeChatEvents).mockImplementation(listeners => {
    captured = listeners;
    return () => {};
  });

  // Mark the pending user's socket as connected so the subscribe effect fires.
  store.dispatch(setStatusForUser({ userId: '__pending__', status: 'connected' }));

  render(
    <Provider store={store}>
      <ChatRuntimeProvider>
        <div />
      </ChatRuntimeProvider>
    </Provider>
  );

  return captured;
}

function resetRuntimeState() {
  // Reset chatRuntime + thread slices to clean state by dispatching a thread
  // selection that clears ambient state.
  store.dispatch(clearAllThreads());
  store.dispatch(clearAllChatRuntime());
  // `clearAllChatRuntime` intentionally preserves cumulative session token
  // usage; reset it here so usage-recording tests stay isolated regardless of
  // run order.
  store.dispatch(resetSessionTokenUsage());
  store.dispatch(setStatusForUser({ userId: '__pending__', status: 'disconnected' }));
}

describe('ChatRuntimeProvider — dedupe, proactive resolution, mid-turn invariants', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetRuntimeState();
    vi.mocked(threadApi.appendMessage).mockImplementation(async (_tid, msg) => msg);
    vi.mocked(threadApi.getThreads).mockResolvedValue({ threads: [], count: 0 });
    vi.mocked(threadApi.generateTitleIfNeeded).mockResolvedValue({
      id: 'tid',
      title: 'new',
    } as never);
  });

  describe('dedupe', () => {
    it('finds pending spawn and async delegation tool rows', () => {
      const entries = [
        { id: 'ignored', name: 'search', round: 0, status: 'running' },
        {
          id: 'spawn',
          name: 'spawn_async_subagent',
          round: 0,
          status: 'running',
          argsBuffer: '{"prompt":"Archive preferences."}',
        },
      ] as Parameters<typeof findPendingDelegationContext>[0];

      expect(findPendingDelegationContext(entries, 0)).toEqual({
        sourceToolName: 'spawn_async_subagent',
        prompt: 'Archive preferences.',
        spawnEntryId: 'spawn',
      });

      expect(
        findPendingDelegationContext(
          [{ id: 'sync', name: 'spawn_subagent', round: 1, status: 'running' }] as Parameters<
            typeof findPendingDelegationContext
          >[0],
          1
        )
      ).toMatchObject({ sourceToolName: 'spawn_subagent', spawnEntryId: 'sync' });
    });

    it('stores task board updates from socket events', () => {
      const listeners = renderProvider();
      const board = {
        threadId: 'thread-board',
        updatedAt: '2026-05-04T10:00:05Z',
        cards: [
          { id: 'task-1', title: 'Plan', status: 'todo' as const, order: 0, updatedAt: 'now' },
        ],
      };

      act(() => {
        listeners.onTaskBoardUpdated?.({
          thread_id: 'thread-board',
          request_id: 'req-board',
          task_board: board,
        });
      });

      expect(store.getState().chatRuntime.taskBoardByThread['thread-board']).toEqual(board);
    });

    it('drops duplicate tool_call events with the same thread/request/round/tool', () => {
      const listeners = renderProvider();

      const event: chatService.ChatToolCallEvent = {
        thread_id: 't1',
        request_id: 'r1',
        round: 0,
        tool_name: 'search',
        skill_id: 'notion',
        args: {},
        tool_call_id: 'call-1',
      };

      act(() => {
        listeners.onToolCall?.(event);
        listeners.onToolCall?.(event);
      });

      const timeline = store.getState().chatRuntime.toolTimelineByThread['t1'] ?? [];
      expect(timeline).toHaveLength(1);
      expect(timeline[0]?.name).toBe('search');
      expect(timeline[0]?.status).toBe('running');
    });

    it('collapses a spawn_subagent tool-call row into the subagent row', () => {
      const listeners = renderProvider();

      act(() => {
        // Parent invokes the delegation tool — creates a "spawn_subagent" row.
        listeners.onToolCall?.({
          thread_id: 't1',
          request_id: 'r1',
          round: 0,
          tool_name: 'spawn_subagent',
          skill_id: 'orchestration',
          args: {},
          tool_call_id: 'call-spawn',
        });
        // The delegation prompt streams in as the tool's args JSON.
        listeners.onToolArgsDelta?.({
          thread_id: 't1',
          request_id: 'r1',
          round: 0,
          tool_call_id: 'call-spawn',
          tool_name: 'spawn_subagent',
          delta: '{"prompt":"Research Q3 revenue."}',
        });
        // The child spawns — should REPLACE the tool-call row, not add a second.
        listeners.onSubagentSpawned?.({
          thread_id: 't1',
          request_id: 'r1',
          round: 0,
          tool_name: 'researcher',
          skill_id: 'sub-1',
          message: 'spawned',
          subagent: { mode: 'typed' },
        });
      });

      const timeline = store.getState().chatRuntime.toolTimelineByThread['t1'] ?? [];
      expect(timeline).toHaveLength(1);
      expect(timeline[0]?.name).toBe('subagent:researcher');
      // The parent's delegation prompt is carried onto the subagent so the
      // drawer can open the conversation with it.
      expect(timeline[0]?.subagent?.prompt).toContain('Research Q3 revenue');
    });

    it('collapses a spawn_async_subagent tool-call row into the subagent row', () => {
      const listeners = renderProvider();

      act(() => {
        listeners.onToolCall?.({
          thread_id: 't1',
          request_id: 'r1',
          round: 0,
          tool_name: 'spawn_async_subagent',
          skill_id: 'orchestration',
          args: {},
          tool_call_id: 'call-spawn-async',
        });
        listeners.onToolArgsDelta?.({
          thread_id: 't1',
          request_id: 'r1',
          round: 0,
          tool_call_id: 'call-spawn-async',
          tool_name: 'spawn_async_subagent',
          delta: '{"prompt":"Archive these preferences."}',
        });
        listeners.onSubagentSpawned?.({
          thread_id: 't1',
          request_id: 'r1',
          round: 0,
          tool_name: 'archivist',
          skill_id: 'sub-async-1',
          message: 'spawned',
          subagent: { mode: 'async' },
        });
      });

      const timeline = store.getState().chatRuntime.toolTimelineByThread['t1'] ?? [];
      expect(timeline).toHaveLength(1);
      expect(timeline[0]?.name).toBe('subagent:archivist');
      expect(timeline[0]?.sourceToolName).toBe('spawn_async_subagent');
      expect(timeline[0]?.subagent?.prompt).toContain('Archive these preferences');
    });

    it('appends streamed subagent text & thinking deltas to the subagent transcript', () => {
      const listeners = renderProvider();

      act(() => {
        listeners.onSubagentSpawned?.({
          thread_id: 't1',
          request_id: 'r1',
          round: 0,
          tool_name: 'researcher',
          skill_id: 'sub-1',
          message: 'spawned',
          subagent: { mode: 'typed' },
        });
        listeners.onSubagentThinkingDelta?.({
          thread_id: 't1',
          request_id: 'r1',
          round: 0,
          delta: 'let me think',
          subagent: { task_id: 'sub-1', agent_id: 'researcher', child_iteration: 1 },
        });
        listeners.onSubagentTextDelta?.({
          thread_id: 't1',
          request_id: 'r1',
          round: 0,
          delta: 'the answer',
          subagent: { task_id: 'sub-1', agent_id: 'researcher', child_iteration: 1 },
        });
      });

      const row = (store.getState().chatRuntime.toolTimelineByThread['t1'] ?? []).find(
        e => e.subagent?.taskId === 'sub-1'
      );
      expect(row?.subagent?.transcript).toEqual([
        { kind: 'thinking', iteration: 1, text: 'let me think' },
        { kind: 'text', iteration: 1, text: 'the answer' },
      ]);
    });

    it('ignores subagent deltas missing task/agent/delta', () => {
      const listeners = renderProvider();
      act(() => {
        listeners.onSubagentSpawned?.({
          thread_id: 't1',
          request_id: 'r1',
          round: 0,
          tool_name: 'researcher',
          skill_id: 'sub-1',
          message: 'spawned',
          subagent: { mode: 'typed' },
        });
        // No agent_id → dropped.
        listeners.onSubagentTextDelta?.({
          thread_id: 't1',
          request_id: 'r1',
          round: 0,
          delta: 'x',
          subagent: { task_id: 'sub-1' },
        });
      });
      const row = (store.getState().chatRuntime.toolTimelineByThread['t1'] ?? []).find(
        e => e.subagent?.taskId === 'sub-1'
      );
      expect(row?.subagent?.transcript).toEqual([]);
    });

    it('routes a parallel (forked) turn into its own lane, leaving the primary stream untouched', () => {
      const listeners = renderProvider();

      // Primary turn streams on the thread.
      act(() => {
        listeners.onTextDelta?.({
          thread_id: 't-par',
          request_id: 'primary',
          round: 0,
          delta: 'P',
        });
      });
      // A parallel turn is registered and streams concurrently on the SAME thread.
      act(() => {
        store.dispatch(registerParallelRequest({ threadId: 't-par', requestId: 'branch' }));
        listeners.onTextDelta?.({
          thread_id: 't-par',
          request_id: 'branch',
          round: 0,
          delta: 'B1',
        });
        listeners.onTextDelta?.({
          thread_id: 't-par',
          request_id: 'branch',
          round: 0,
          delta: 'B2',
        });
      });

      const mid = store.getState().chatRuntime;
      // Primary stream is not clobbered by the parallel branch.
      expect(mid.streamingAssistantByThread['t-par']?.content).toBe('P');
      expect(mid.parallelStreamsByThread['t-par']?.['branch']?.content).toBe('B1B2');

      // The parallel turn's chat_done resolves ONLY its lane; the primary
      // stream and its (still-running) state survive.
      act(() => {
        listeners.onDone?.({
          thread_id: 't-par',
          request_id: 'branch',
          full_response: 'branch done',
          rounds_used: 1,
          total_input_tokens: 0,
          total_output_tokens: 0,
          segment_total: 0,
        });
      });

      const after = store.getState().chatRuntime;
      expect(after.parallelStreamsByThread['t-par']).toBeUndefined();
      expect(after.parallelRequestThreads['branch']).toBeUndefined();
      expect(after.streamingAssistantByThread['t-par']?.content).toBe('P');
    });

    it('drops duplicate chat_done events with the same thread/request', async () => {
      const listeners = renderProvider();

      const doneEvent: chatService.ChatDoneEvent = {
        thread_id: 't-done',
        request_id: 'r-done',
        full_response: 'hello',
        rounds_used: 1,
        total_input_tokens: 5,
        total_output_tokens: 7,
        segment_total: 1,
      };

      act(() => {
        listeners.onDone?.(doneEvent);
        listeners.onDone?.(doneEvent);
      });

      // Usage recorded exactly once despite duplicate dispatch.
      const usage = store.getState().chatRuntime.sessionTokenUsage;
      expect(usage.inputTokens).toBe(5);
      expect(usage.outputTokens).toBe(7);
      expect(usage.turns).toBe(1);

      // Snapshot refetch fired exactly once on the first chat_done — issue #924.
      await waitFor(() => expect(mockRefetchSnapshot).toHaveBeenCalledTimes(1));
    });

    it('stores a parked plan review from the plan_review_request event', () => {
      const listeners = renderProvider();
      act(() => {
        listeners.onPlanReviewRequest?.({
          thread_id: 't-plan',
          request_id: 'pr-1',
          message: 'Ship the release',
          args: { steps: ['build', 'verify'] },
        });
      });
      const review = store.getState().chatRuntime.pendingPlanReviewByThread['t-plan'];
      expect(review).toEqual({
        requestId: 'pr-1',
        summary: 'Ship the release',
        steps: ['build', 'verify'],
      });
    });

    it('clears a parked plan review when the turn ends or errors', () => {
      const listeners = renderProvider();
      const park = () =>
        store.dispatch(
          setPendingPlanReviewForThread({
            threadId: 't-plan',
            review: { requestId: 'pr-1', summary: 'Plan', steps: ['a'] },
          })
        );

      // chat_done clears the (possibly expired) parked review.
      park();
      act(() => {
        listeners.onDone?.({
          thread_id: 't-plan',
          request_id: 'r-plan',
          full_response: 'done',
          rounds_used: 1,
          total_input_tokens: 1,
          total_output_tokens: 1,
          segment_total: 1,
        });
      });
      expect(store.getState().chatRuntime.pendingPlanReviewByThread['t-plan']).toBeUndefined();

      // chat_error also clears it (cancelled/errored parked turn).
      park();
      act(() => {
        listeners.onError?.({
          thread_id: 't-plan',
          request_id: 'r-plan',
          message: 'boom',
          error_type: 'inference',
          round: 1,
        });
      });
      expect(store.getState().chatRuntime.pendingPlanReviewByThread['t-plan']).toBeUndefined();
    });

    it('flushes queued follow-ups into the transcript when a turn ends', async () => {
      const listeners = renderProvider();
      store.dispatch(
        enqueueFollowup({
          threadId: 't-fup',
          message: {
            id: 'f1',
            content: 'queued follow-up text',
            type: 'text',
            extraMetadata: {},
            sender: 'user',
            createdAt: '2026-01-01T00:00:00.000Z',
          },
          label: 'queued follow-up text',
        })
      );

      await act(async () => {
        listeners.onDone?.({
          thread_id: 't-fup',
          request_id: 'r-fup',
          full_response: 'assistant reply',
          rounds_used: 1,
          total_input_tokens: 1,
          total_output_tokens: 1,
        });
      });

      // The queued prompt is persisted as a real user message (survives reload,
      // appended AFTER the assistant reply) and the pills are cleared.
      await waitFor(() =>
        expect(threadApi.appendMessage).toHaveBeenCalledWith(
          't-fup',
          expect.objectContaining({ content: 'queued follow-up text', sender: 'user' })
        )
      );
      expect(store.getState().chatRuntime.queuedFollowupsByThread['t-fup']).toBeUndefined();
    });

    it('processes tool_call for different rounds as distinct events', () => {
      const listeners = renderProvider();

      act(() => {
        listeners.onToolCall?.({
          thread_id: 't1',
          request_id: 'r1',
          round: 0,
          tool_name: 'search',
          skill_id: 'notion',
          args: {},
          tool_call_id: 'call-1',
        });
        listeners.onToolCall?.({
          thread_id: 't1',
          request_id: 'r1',
          round: 1,
          tool_name: 'search',
          skill_id: 'notion',
          args: {},
          tool_call_id: 'call-2',
        });
      });

      const timeline = store.getState().chatRuntime.toolTimelineByThread['t1'] ?? [];
      expect(timeline).toHaveLength(2);
      expect(timeline.map(e => e.round)).toEqual([0, 1]);
    });
  });

  describe('proactive thread resolution', () => {
    it('reuses the selected thread when it is fresh (no messages)', async () => {
      store.dispatch(
        loadThreads.fulfilled(
          { threads: [{ id: 'visible-thread', title: 'x', messageCount: 0 }] as never, count: 1 },
          'req-id',
          undefined
        )
      );
      store.dispatch(setSelectedThread('visible-thread'));
      const listeners = renderProvider();

      await act(async () => {
        listeners.onProactiveMessage?.({
          thread_id: 'proactive:worker-1',
          request_id: 'req-p1',
          full_response: 'ping',
        });
      });

      // createNewThread must NOT be invoked when a fresh visible thread exists.
      expect(threadApi.createNewThread).not.toHaveBeenCalled();
      await waitFor(() =>
        expect(threadApi.appendMessage).toHaveBeenCalledWith(
          'visible-thread',
          expect.objectContaining({ content: 'ping', sender: 'agent' })
        )
      );
    });

    it('opens a new thread instead of interrupting a selected thread that has messages (#3713)', async () => {
      vi.mocked(threadApi.createNewThread).mockResolvedValue({
        id: 'fresh-thread',
        title: 'new',
      } as never);
      vi.mocked(threadApi.getThreads).mockResolvedValue({
        threads: [{ id: 'fresh-thread', title: 'new' }] as never,
        count: 1,
      });

      // The user is mid-conversation: the selected thread already holds
      // messages, so a proactive morning brief / subconscious update must
      // NOT be injected into it.
      store.dispatch(
        loadThreads.fulfilled(
          { threads: [{ id: 'busy-thread', title: 'chat', messageCount: 4 }] as never, count: 1 },
          'req-id',
          undefined
        )
      );
      store.dispatch(setSelectedThread('busy-thread'));
      const listeners = renderProvider();

      await act(async () => {
        listeners.onProactiveMessage?.({
          thread_id: 'proactive:morning_briefing',
          request_id: 'req-mb',
          full_response: "good morning! here's your briefing",
        });
      });

      // The active conversation must be left untouched; delivery goes to a
      // dedicated new thread.
      await waitFor(() => expect(threadApi.createNewThread).toHaveBeenCalledTimes(1));
      expect(threadApi.appendMessage).toHaveBeenCalledWith(
        'fresh-thread',
        expect.objectContaining({ content: "good morning! here's your briefing" })
      );
      expect(threadApi.appendMessage).not.toHaveBeenCalledWith('busy-thread', expect.anything());
    });

    it('treats a selected thread with unknown metadata as occupied and opens a new thread', async () => {
      vi.mocked(threadApi.createNewThread).mockResolvedValue({
        id: 'fresh-thread',
        title: 'new',
      } as never);
      vi.mocked(threadApi.getThreads).mockResolvedValue({
        threads: [{ id: 'fresh-thread', title: 'new' }] as never,
        count: 1,
      });

      // Only a rehydrated selection is present — the thread list hasn't loaded,
      // so its message metadata is unknown. We must NOT assume it is fresh
      // (it could already hold a server-side conversation). See #3713.
      store.dispatch(setSelectedThread('rehydrated-thread'));
      const listeners = renderProvider();

      await act(async () => {
        listeners.onProactiveMessage?.({
          thread_id: 'proactive:morning_briefing',
          request_id: 'req-unknown',
          full_response: 'briefing',
        });
      });

      await waitFor(() => expect(threadApi.createNewThread).toHaveBeenCalledTimes(1));
      expect(threadApi.appendMessage).toHaveBeenCalledWith(
        'fresh-thread',
        expect.objectContaining({ content: 'briefing' })
      );
      expect(threadApi.appendMessage).not.toHaveBeenCalledWith(
        'rehydrated-thread',
        expect.anything()
      );
    });

    it('creates a new thread when no visible thread exists for proactive handoff', async () => {
      vi.mocked(threadApi.createNewThread).mockResolvedValue({
        id: 'created-thread',
        title: 'new',
      } as never);
      vi.mocked(threadApi.getThreads).mockResolvedValue({
        threads: [{ id: 'created-thread', title: 'new' }] as never,
        count: 1,
      });

      const listeners = renderProvider();

      await act(async () => {
        listeners.onProactiveMessage?.({
          thread_id: 'proactive:worker-2',
          request_id: 'req-p2',
          full_response: 'bootstrap msg',
        });
      });

      await waitFor(() => expect(threadApi.appendMessage).toHaveBeenCalled());
      expect(threadApi.createNewThread).toHaveBeenCalledTimes(1);
      expect(threadApi.appendMessage).toHaveBeenCalledWith(
        'created-thread',
        expect.objectContaining({ content: 'bootstrap msg' })
      );
    });

    it('deduplicates identical proactive messages from the same sender', async () => {
      store.dispatch(
        loadThreads.fulfilled(
          { threads: [{ id: 'visible-thread', title: 'x' }] as never, count: 1 },
          'req-id',
          undefined
        )
      );
      store.dispatch(setSelectedThread('visible-thread'));
      const listeners = renderProvider();

      const event: chatService.ProactiveMessageEvent = {
        thread_id: 'proactive:worker-3',
        request_id: 'req-dup',
        full_response: 'ping',
      };

      await act(async () => {
        listeners.onProactiveMessage?.(event);
        listeners.onProactiveMessage?.(event);
      });

      await waitFor(() => expect(threadApi.appendMessage).toHaveBeenCalledTimes(1));
    });
  });

  describe('mid-turn streaming invariants', () => {
    it('reconciles missing segment events from chat_done.full_response', async () => {
      const listeners = renderProvider();

      act(() => {
        listeners.onSegment?.({
          thread_id: 't-segmented',
          request_id: 'r-segmented',
          full_response: 'Part one.',
          segment_index: 0,
          segment_total: 2,
        });
      });

      await waitFor(() =>
        expect(threadApi.appendMessage).toHaveBeenCalledWith(
          't-segmented',
          expect.objectContaining({ content: 'Part one.', sender: 'agent' })
        )
      );

      act(() => {
        listeners.onDone?.({
          thread_id: 't-segmented',
          request_id: 'r-segmented',
          full_response: 'Part one.\n\nPart two.',
          rounds_used: 1,
          total_input_tokens: 10,
          total_output_tokens: 20,
          segment_total: 2,
        });
      });

      await waitFor(() =>
        expect(threadApi.appendMessage).toHaveBeenCalledWith(
          't-segmented',
          expect.objectContaining({ content: 'Part one.\n\nPart two.', sender: 'agent' })
        )
      );
      expect(threadApi.appendMessage).toHaveBeenCalledTimes(2);
    });

    it('does not duplicate chat_done.full_response when all segments arrived', async () => {
      const listeners = renderProvider();

      act(() => {
        listeners.onSegment?.({
          thread_id: 't-complete',
          request_id: 'r-complete',
          full_response: 'Part one.\n\n',
          segment_index: 0,
          segment_total: 2,
        });
        listeners.onSegment?.({
          thread_id: 't-complete',
          request_id: 'r-complete',
          full_response: 'Part two.',
          segment_index: 1,
          segment_total: 2,
        });
      });

      await waitFor(() => expect(threadApi.appendMessage).toHaveBeenCalledTimes(2));

      act(() => {
        listeners.onDone?.({
          thread_id: 't-complete',
          request_id: 'r-complete',
          full_response: 'Part one.\n\nPart two.',
          rounds_used: 1,
          total_input_tokens: 10,
          total_output_tokens: 20,
          segment_total: 2,
        });
      });

      await waitFor(() => expect(mockRefetchSnapshot).toHaveBeenCalledTimes(1));
      expect(threadApi.appendMessage).toHaveBeenCalledTimes(2);
    });

    it('does not reconcile when all segments arrived even if full_response differs (trim/joiner)', async () => {
      // Regression: the server's segmenter trims each segment and joins with
      // "\n\n", while chat_done.full_response is the raw LLM text. A strict
      // byte-equality check used to fire reconciliation on every multi-segment
      // turn — once we have all expected segment_index values, delivery is
      // complete regardless of full_response content.
      const listeners = renderProvider();

      act(() => {
        listeners.onSegment?.({
          thread_id: 't-trim',
          request_id: 'r-trim',
          full_response: 'Hello.',
          segment_index: 0,
          segment_total: 2,
        });
        listeners.onSegment?.({
          thread_id: 't-trim',
          request_id: 'r-trim',
          full_response: 'World.',
          segment_index: 1,
          segment_total: 2,
        });
      });

      await waitFor(() => expect(threadApi.appendMessage).toHaveBeenCalledTimes(2));

      act(() => {
        listeners.onDone?.({
          thread_id: 't-trim',
          request_id: 'r-trim',
          // Raw LLM output: leading/trailing whitespace + paragraph break.
          // segments.join('') === 'Hello.World.' !== full_response, but
          // delivery is still complete.
          full_response: '\nHello.\n\nWorld.\n',
          rounds_used: 1,
          total_input_tokens: 10,
          total_output_tokens: 20,
          segment_total: 2,
        });
      });

      await waitFor(() => expect(mockRefetchSnapshot).toHaveBeenCalledTimes(1));
      expect(threadApi.appendMessage).toHaveBeenCalledTimes(2);
    });

    it('reconciles when a segment is missing', async () => {
      const listeners = renderProvider();

      act(() => {
        listeners.onSegment?.({
          thread_id: 't-missing',
          request_id: 'r-missing',
          full_response: 'Part one.',
          segment_index: 0,
          segment_total: 2,
        });
        // segment_index 1 never arrives.
      });

      await waitFor(() => expect(threadApi.appendMessage).toHaveBeenCalledTimes(1));

      act(() => {
        listeners.onDone?.({
          thread_id: 't-missing',
          request_id: 'r-missing',
          full_response: 'Part one.\n\nPart two.',
          rounds_used: 1,
          total_input_tokens: 10,
          total_output_tokens: 20,
          segment_total: 2,
        });
      });

      await waitFor(() =>
        expect(threadApi.appendMessage).toHaveBeenCalledWith(
          't-missing',
          expect.objectContaining({ content: 'Part one.\n\nPart two.', sender: 'agent' })
        )
      );
      expect(threadApi.appendMessage).toHaveBeenCalledTimes(2);
    });

    it('expires stale segment delivery state before chat_done reconciliation', async () => {
      const nowSpy = vi.spyOn(Date, 'now').mockReturnValue(1_000);
      const listeners = renderProvider();

      try {
        act(() => {
          listeners.onSegment?.({
            thread_id: 't-stale',
            request_id: 'r-stale',
            full_response: 'Stale segment.',
            segment_index: 0,
            segment_total: 1,
          });
        });

        await waitFor(() => expect(threadApi.appendMessage).toHaveBeenCalledTimes(1));
        nowSpy.mockReturnValue(1_000 + 5 * 60 * 1000 + 1);

        act(() => {
          listeners.onDone?.({
            thread_id: 't-stale',
            request_id: 'r-stale',
            full_response: 'Stale segment.',
            rounds_used: 1,
            total_input_tokens: 10,
            total_output_tokens: 20,
            segment_total: 1,
          });
        });

        await waitFor(() => expect(threadApi.appendMessage).toHaveBeenCalledTimes(2));
      } finally {
        nowSpy.mockRestore();
      }
    });

    it('accumulates text_delta chunks within the same request_id', () => {
      const listeners = renderProvider();

      act(() => {
        listeners.onTextDelta?.({ thread_id: 't-mid', request_id: 'r1', round: 0, delta: 'Hel' });
        listeners.onTextDelta?.({ thread_id: 't-mid', request_id: 'r1', round: 0, delta: 'lo!' });
      });

      const streaming = store.getState().chatRuntime.streamingAssistantByThread['t-mid'];
      expect(streaming).toBeDefined();
      expect(streaming?.requestId).toBe('r1');
      expect(streaming?.content).toBe('Hello!');
    });

    it('replaces streaming state when request_id changes mid-turn', () => {
      const listeners = renderProvider();

      act(() => {
        listeners.onTextDelta?.({ thread_id: 't-mid', request_id: 'r1', round: 0, delta: 'aaa' });
        listeners.onTextDelta?.({ thread_id: 't-mid', request_id: 'r2', round: 0, delta: 'bbb' });
      });

      const streaming = store.getState().chatRuntime.streamingAssistantByThread['t-mid'];
      expect(streaming?.requestId).toBe('r2');
      expect(streaming?.content).toBe('bbb');
    });

    it('sets inference status to thinking on inference_start and clears it on chat_done', () => {
      const listeners = renderProvider();

      act(() => {
        listeners.onInferenceStart?.({ thread_id: 't-inv', request_id: 'r1' });
      });
      expect(store.getState().chatRuntime.inferenceStatusByThread['t-inv']?.phase).toBe('thinking');

      act(() => {
        listeners.onDone?.({
          thread_id: 't-inv',
          request_id: 'r1',
          full_response: '',
          rounds_used: 1,
          total_input_tokens: 0,
          total_output_tokens: 0,
        });
      });
      expect(store.getState().chatRuntime.inferenceStatusByThread['t-inv']).toBeUndefined();
      expect(store.getState().chatRuntime.streamingAssistantByThread['t-inv']).toBeUndefined();
    });

    it('terminates running tool-timeline rows on chat_done', () => {
      const listeners = renderProvider();

      act(() => {
        listeners.onToolCall?.({
          thread_id: 't-inv',
          request_id: 'r1',
          round: 0,
          tool_name: 'search',
          skill_id: 'notion',
          args: {},
          tool_call_id: 'call-1',
        });
      });
      expect(store.getState().chatRuntime.toolTimelineByThread['t-inv']?.[0]?.status).toBe(
        'running'
      );

      act(() => {
        listeners.onDone?.({
          thread_id: 't-inv',
          request_id: 'r1',
          full_response: '',
          rounds_used: 1,
          total_input_tokens: 0,
          total_output_tokens: 0,
        });
      });

      const timeline = store.getState().chatRuntime.toolTimelineByThread['t-inv'] ?? [];
      expect(timeline).toHaveLength(1);
      expect(timeline[0]?.status).toBe('success');
    });

    it('transitions running tool-timeline rows to error on chat_error', () => {
      const listeners = renderProvider();

      act(() => {
        listeners.onToolCall?.({
          thread_id: 't-err',
          request_id: 'r1',
          round: 0,
          tool_name: 'search',
          skill_id: 'notion',
          args: {},
          tool_call_id: 'call-err',
        });
      });

      act(() => {
        listeners.onError?.({
          thread_id: 't-err',
          request_id: 'r1',
          message: 'timeout',
          error_type: 'timeout',
          round: 0,
        });
      });

      const timeline = store.getState().chatRuntime.toolTimelineByThread['t-err'] ?? [];
      expect(timeline[0]?.status).toBe('error');
      expect(store.getState().chatRuntime.inferenceStatusByThread['t-err']).toBeUndefined();
    });

    it('forwards the server-provided inference error message verbatim', async () => {
      const listeners = renderProvider();
      // Transport-level failures yield no provider `error.message` body, so
      // `with_provider_detail()` in web_errors.rs returns just the friendly
      // generic message with no raw URL appended — the FE forwards it as-is
      // (backend owns sanitization; see web_errors_tests.rs).
      const serverMessage =
        'Something went wrong. Please try again.\nThis error has been reported. You can also report it on Discord.\n<openhuman-link path="community/discord-report">Report on Discord</openhuman-link>';

      act(() => {
        listeners.onError?.({
          thread_id: 't-err-sanitized',
          request_id: 'r1',
          message: serverMessage,
          error_type: 'inference',
          round: 0,
        });
      });

      await waitFor(() =>
        expect(threadApi.appendMessage).toHaveBeenCalledWith(
          't-err-sanitized',
          expect.objectContaining({ sender: 'agent', content: serverMessage })
        )
      );
    });

    it('does not append a duplicate error bubble when the previous message already matches', async () => {
      const listeners = renderProvider();
      const repeated = 'Your AI provider is temporarily unavailable. Please try again later.';

      act(() => {
        listeners.onError?.({
          thread_id: 't-err-dedupe',
          request_id: 'r1',
          message: repeated,
          error_type: 'inference',
          round: 0,
        });
      });

      await waitFor(() =>
        expect(threadApi.appendMessage).toHaveBeenCalledWith(
          't-err-dedupe',
          expect.objectContaining({ content: repeated })
        )
      );

      act(() => {
        listeners.onError?.({
          thread_id: 't-err-dedupe',
          request_id: 'r2',
          message: repeated,
          error_type: 'inference',
          round: 0,
        });
      });

      await waitFor(() => {
        const matchingCalls = vi
          .mocked(threadApi.appendMessage)
          .mock.calls.filter(
            call =>
              call[0] === 't-err-dedupe' &&
              typeof call[1]?.content === 'string' &&
              call[1].content.includes(repeated)
          );
        expect(matchingCalls).toHaveLength(1);
      });
    });
  });

  // Live subagent activity (#1122) — the parent thread surfaces a
  // subagent's child iterations and tool calls as they happen, then
  // settles to the final-run statistics on completion. The asserts here
  // are the contract the ToolTimelineBlock UI relies on; if a refactor
  // moves the subagent state somewhere else this test is the canary.
  describe('live subagent activity (#1122)', () => {
    it('builds a live subagent block from spawned → iteration → tool call → done', () => {
      const listeners = renderProvider();
      const threadId = 'tsa';

      act(() => {
        listeners.onSubagentSpawned?.({
          thread_id: threadId,
          request_id: 'r1',
          tool_name: 'researcher',
          skill_id: 'sub-1',
          message: 'spawned',
          round: 1,
          subagent: { mode: 'typed', dedicated_thread: false, prompt_chars: 42 },
        });
      });

      let timeline = store.getState().chatRuntime.toolTimelineByThread[threadId] ?? [];
      expect(timeline).toHaveLength(1);
      expect(timeline[0]?.subagent).toMatchObject({
        agentId: 'researcher',
        taskId: 'sub-1',
        mode: 'typed',
        dedicatedThread: false,
        toolCalls: [],
      });

      act(() => {
        listeners.onSubagentIterationStart?.({
          thread_id: threadId,
          request_id: 'r1',
          round: 1,
          tool_name: 'researcher',
          skill_id: 'sub-1',
          message: 'iter',
          subagent: {
            agent_id: 'researcher',
            task_id: 'sub-1',
            child_iteration: 1,
            child_max_iterations: 5,
          },
        });
        listeners.onSubagentToolCall?.({
          thread_id: threadId,
          request_id: 'r1',
          round: 1,
          tool_name: 'web_search',
          skill_id: 'sub-1',
          tool_call_id: 'cc-1',
          subagent: { agent_id: 'researcher', task_id: 'sub-1', child_iteration: 1 },
        });
        // Duplicate child tool_call must not double-append.
        listeners.onSubagentToolCall?.({
          thread_id: threadId,
          request_id: 'r1',
          round: 1,
          tool_name: 'web_search',
          skill_id: 'sub-1',
          tool_call_id: 'cc-1',
          subagent: { agent_id: 'researcher', task_id: 'sub-1', child_iteration: 1 },
        });
      });

      timeline = store.getState().chatRuntime.toolTimelineByThread[threadId] ?? [];
      expect(timeline[0]?.subagent?.childIteration).toBe(1);
      expect(timeline[0]?.subagent?.childMaxIterations).toBe(5);
      expect(timeline[0]?.subagent?.toolCalls).toEqual([
        { callId: 'cc-1', toolName: 'web_search', status: 'running', iteration: 1 },
      ]);

      act(() => {
        listeners.onSubagentToolResult?.({
          thread_id: threadId,
          request_id: 'r1',
          round: 1,
          tool_name: 'web_search',
          skill_id: 'sub-1',
          tool_call_id: 'cc-1',
          success: true,
          subagent: {
            agent_id: 'researcher',
            task_id: 'sub-1',
            child_iteration: 1,
            elapsed_ms: 312,
            output_chars: 1280,
          },
        });
        listeners.onSubagentDone?.({
          thread_id: threadId,
          request_id: 'r1',
          tool_name: 'researcher',
          skill_id: 'sub-1',
          message: 'done',
          success: true,
          round: 1,
          subagent: { iterations: 2, elapsed_ms: 4200, output_chars: 980 },
        });
      });

      timeline = store.getState().chatRuntime.toolTimelineByThread[threadId] ?? [];
      expect(timeline[0]?.status).toBe('success');
      expect(timeline[0]?.subagent?.toolCalls[0]).toMatchObject({
        status: 'success',
        elapsedMs: 312,
        outputChars: 1280,
      });
      expect(timeline[0]?.subagent).toMatchObject({
        iterations: 2,
        elapsedMs: 4200,
        outputChars: 980,
      });
    });

    it('ignores subagent_tool_call events that arrive before subagent_spawned', () => {
      const listeners = renderProvider();
      const threadId = 'tsa-orphan';

      act(() => {
        listeners.onSubagentToolCall?.({
          thread_id: threadId,
          request_id: 'r1',
          round: 1,
          tool_name: 'web_search',
          skill_id: 'sub-missing',
          tool_call_id: 'cc-1',
          subagent: { agent_id: 'researcher', task_id: 'sub-missing', child_iteration: 1 },
        });
      });

      // No row was created — the orphan child tool call is dropped rather
      // than synthesising a partial subagent row from incomplete data.
      const timeline = store.getState().chatRuntime.toolTimelineByThread[threadId] ?? [];
      expect(timeline).toHaveLength(0);
    });
  });

  // Regression: on Windows users report being "locked out" of the composer
  // after sleep/wake or a network flap — the in-flight turn's `chat_done`
  // never arrives on the new socket, so `activeThreadId` and the
  // `started`/`streaming` lifecycle stay set and the textarea remains
  // disabled. The reconcile effect releases them on disconnect so the
  // composer becomes typeable again immediately.
  describe('socket-disconnect reconciliation (#WindowsLockout)', () => {
    it('clears activeThreadId and in-flight lifecycle when the socket drops mid-turn', async () => {
      const threadId = 't-disconnect';
      renderProvider();

      await act(async () => {
        const { setActiveThread } = await import('../../store/threadSlice');
        const { beginInferenceTurn, setInferenceStatusForThread } =
          await import('../../store/chatRuntimeSlice');
        store.dispatch(setActiveThread(threadId));
        store.dispatch(beginInferenceTurn({ threadId }));
        store.dispatch(
          setInferenceStatusForThread({
            threadId,
            status: { phase: 'thinking', iteration: 1, maxIterations: 10 },
          })
        );
      });

      expect(store.getState().thread.activeThreadIds[threadId]).toBe(true);
      expect(store.getState().chatRuntime.inferenceTurnLifecycleByThread[threadId]).toBe('started');

      await act(async () => {
        store.dispatch(setStatusForUser({ userId: '__pending__', status: 'disconnected' }));
      });

      await waitFor(() => {
        expect(store.getState().thread.activeThreadIds[threadId]).toBeUndefined();
        expect(
          store.getState().chatRuntime.inferenceTurnLifecycleByThread[threadId]
        ).toBeUndefined();
        expect(store.getState().chatRuntime.inferenceStatusByThread[threadId]).toBeUndefined();
      });
    });

    it('preserves streamingAssistant text so the partial reply stays visible after disconnect', async () => {
      const threadId = 't-disconnect-streaming';
      renderProvider();

      await act(async () => {
        const { setActiveThread } = await import('../../store/threadSlice');
        const { beginInferenceTurn, setStreamingAssistantForThread } =
          await import('../../store/chatRuntimeSlice');
        store.dispatch(setActiveThread(threadId));
        store.dispatch(beginInferenceTurn({ threadId }));
        store.dispatch(
          setStreamingAssistantForThread({
            threadId,
            streaming: { requestId: 'req-1', content: 'Hello there, partial', thinking: '' },
          })
        );
      });

      await act(async () => {
        store.dispatch(setStatusForUser({ userId: '__pending__', status: 'disconnected' }));
      });

      await waitFor(() => {
        expect(store.getState().thread.activeThreadIds[threadId]).toBeUndefined();
      });
      expect(store.getState().chatRuntime.streamingAssistantByThread[threadId]).toMatchObject({
        content: 'Hello there, partial',
      });
    });
  });

  // Error classifier full set — Batch-5 coverage (#1506, pr#1566).
  //
  // Every error_type — including the generic 'inference' fallback — carries a
  // user-friendly `message` from classify_inference_error() in web_errors.rs,
  // which is forwarded directly so the user sees the real reason (for
  // 'inference' that message is a friendly summary plus the sanitized upstream
  // provider error as a `> quote` block). The USER_FACING_FALLBACK constant is
  // only used when the server sends an empty/missing message. 'cancelled'
  // produces no bubble at all.
  describe('inference error classifier — full type set', () => {
    const USER_FACING_FALLBACK =
      'Something went wrong. Please try again.\nThis error has been reported. You can also report it on Discord.\n<openhuman-link path="community/discord-report">Report on Discord</openhuman-link>';

    it.each([
      ['rate_limited', 'You have been rate limited. Please try again later.'],
      ['auth_error', 'Authentication failed. Please reconnect your account.'],
      ['budget_exhausted', 'Your usage budget has been exhausted.'],
      ['context_overflow', 'The conversation is too long. Please start a new thread.'],
      ['timeout', 'The request timed out. Please try again.'],
      ['network', 'A network error occurred. Please check your connection.'],
      ['tool_error', 'A tool call failed during this request.'],
      ['provider_error', 'The AI provider returned an error.'],
      ['model_unavailable', 'The selected model is currently unavailable.'],
      [
        'payload_too_large',
        'Your message or attachment is too large for this model. Shorten it or remove the attachment — or start a new thread.',
      ],
    ] as const)('forwards server message for error_type %s', async (error_type, serverMessage) => {
      const listeners = renderProvider();
      const threadId = `t-${error_type}`;

      act(() => {
        listeners.onError?.({
          thread_id: threadId,
          request_id: 'r1',
          message: serverMessage,
          error_type,
          round: 0,
        });
      });

      await waitFor(() =>
        expect(threadApi.appendMessage).toHaveBeenCalledWith(
          threadId,
          expect.objectContaining({ content: serverMessage, sender: 'agent' })
        )
      );
    });

    it('surfaces the server inference message (friendly summary + sanitized provider detail)', async () => {
      const listeners = renderProvider();
      const threadId = 't-inference-detail';
      // Shape produced by web_errors.rs `with_provider_detail(generic, err)`:
      // the friendly summary, then the real upstream reason as a `> quote`
      // block (already secret-scrubbed and length-capped server-side).
      const serverMessage =
        'Something went wrong. Please try again.\n\n> Project `proj_x` does not have access to model `gpt-5.5`.';

      act(() => {
        listeners.onError?.({
          thread_id: threadId,
          request_id: 'r1',
          message: serverMessage,
          error_type: 'inference',
          round: 0,
        });
      });

      await waitFor(() =>
        expect(threadApi.appendMessage).toHaveBeenCalledWith(
          threadId,
          expect.objectContaining({ content: serverMessage, sender: 'agent' })
        )
      );
    });

    it('falls back to the constant when an inference error has no message', async () => {
      const listeners = renderProvider();
      const threadId = 't-inference-empty';

      act(() => {
        listeners.onError?.({
          thread_id: threadId,
          request_id: 'r1',
          message: '',
          error_type: 'inference',
          round: 0,
        });
      });

      await waitFor(() =>
        expect(threadApi.appendMessage).toHaveBeenCalledWith(
          threadId,
          expect.objectContaining({ content: USER_FACING_FALLBACK, sender: 'agent' })
        )
      );
    });

    it('produces no error bubble for cancelled turns', async () => {
      const listeners = renderProvider();
      const threadId = 't-cancelled';

      act(() => {
        listeners.onError?.({
          thread_id: threadId,
          request_id: 'r1',
          message: 'request cancelled by user',
          error_type: 'cancelled',
          round: 0,
        });
      });

      // Give a tick for any async work to complete.
      await new Promise(resolve => setTimeout(resolve, 50));
      expect(threadApi.appendMessage).not.toHaveBeenCalledWith(
        threadId,
        expect.objectContaining({ sender: 'agent' })
      );
    });

    it('falls back to USER_FACING constant when inference error has empty message', async () => {
      const listeners = renderProvider();
      const threadId = 't-empty-msg';

      act(() => {
        listeners.onError?.({
          thread_id: threadId,
          request_id: 'r1',
          message: '',
          error_type: 'network',
          round: 0,
        });
      });

      await waitFor(() =>
        expect(threadApi.appendMessage).toHaveBeenCalledWith(
          threadId,
          expect.objectContaining({ content: USER_FACING_FALLBACK, sender: 'agent' })
        )
      );
    });
  });
});
