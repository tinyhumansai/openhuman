import { beforeEach, describe, expect, it, vi } from 'vitest';

import { setActiveThread } from '../../store/threadSlice';

const mockDispatch = vi.fn();
const mockGetState = vi.fn();
const mockSubscribeChatEvents = vi.fn();
const mockCleanup = vi.fn();

vi.mock('../chatService', () => ({
  subscribeChatEvents: (listeners: unknown) => mockSubscribeChatEvents(listeners),
  segmentText: (event: { full_response: string }) => event.full_response,
}));

vi.mock('../../store', () => ({
  store: { dispatch: (action: unknown) => mockDispatch(action), getState: () => mockGetState() },
}));

describe('chatEventManager', () => {
  beforeEach(() => {
    mockDispatch.mockReset();
    mockGetState.mockReset();
    mockSubscribeChatEvents.mockReset();
    mockCleanup.mockReset();
    mockGetState.mockReturnValue({
      inference: {
        inferenceStatusByThread: {},
        toolTimelineByThread: {},
        streamingAssistantByThread: {},
      },
      thread: { messagesByThreadId: {} },
    });
    mockSubscribeChatEvents.mockReturnValue(mockCleanup);
  });

  it('subscribes once and tears down listeners', async () => {
    const { chatEventManager } = await import('../chatEventManager');

    chatEventManager.init();
    chatEventManager.init();
    expect(mockSubscribeChatEvents).toHaveBeenCalledTimes(1);

    chatEventManager.teardown();
    expect(mockCleanup).toHaveBeenCalledTimes(1);
  });

  it('clears active thread when chat_done arrives', async () => {
    const { chatEventManager } = await import('../chatEventManager');
    chatEventManager.init();

    const listeners = mockSubscribeChatEvents.mock.calls[0][0] as {
      onDone?: (event: {
        thread_id: string;
        request_id: string;
        full_response: string;
        segment_total?: number | null;
      }) => void;
    };

    listeners.onDone?.({
      thread_id: 'thread-1',
      request_id: 'req-1',
      full_response: 'Hello world',
      segment_total: 0,
    });

    expect(mockDispatch).toHaveBeenCalledWith(setActiveThread(null));
  });
});
