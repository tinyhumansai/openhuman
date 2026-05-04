import { render, waitFor } from '@testing-library/react';
import { act } from 'react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import * as chatService from '../../services/chatService';
import { threadApi } from '../../services/api/threadApi';
import { store } from '../../store';
import { clearAllChatRuntime } from '../../store/chatRuntimeSlice';
import { setStatusForUser } from '../../store/socketSlice';
import { clearAllThreads, loadThreads, setSelectedThread } from '../../store/threadSlice';
import ChatRuntimeProvider from '../ChatRuntimeProvider';

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
      expect(mockRefetchSnapshot).toHaveBeenCalledTimes(1);
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
    it('reuses the selected thread when resolving a proactive: sender', async () => {
      store.dispatch(
        loadThreads.fulfilled(
          { threads: [{ id: 'visible-thread', title: 'x' }] as never, count: 1 },
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

      // createNewThread must NOT be invoked when a visible thread already exists.
      expect(threadApi.createNewThread).not.toHaveBeenCalled();
      await waitFor(() =>
        expect(threadApi.appendMessage).toHaveBeenCalledWith(
          'visible-thread',
          expect.objectContaining({ content: 'ping', sender: 'agent' })
        )
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
});
