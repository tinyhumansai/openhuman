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

    it('drops duplicate chat_done events with the same thread/request', () => {
      const listeners = renderProvider();

      const doneEvent: chatService.ChatDoneEvent = {
        thread_id: 't-done',
        request_id: 'r-done',
        full_response: 'hello',
        rounds_used: 1,
        total_input_tokens: 5,
        total_output_tokens: 7,
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
});
