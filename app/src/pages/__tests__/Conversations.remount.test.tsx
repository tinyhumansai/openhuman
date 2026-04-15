// @vitest-environment jsdom
import { render, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockDispatch = vi.fn(() => Promise.resolve());
const mockNavigate = vi.fn();
const mockLoadThreadMessages = vi.fn((threadId: string) => ({
  type: 'thread/loadThreadMessages',
  payload: threadId,
}));

const mockState = {
  thread: {
    selectedThreadId: 'thread-1',
    messages: [],
    isLoadingMessages: false,
    messagesError: null,
    suggestedQuestions: [],
    isLoadingSuggestions: false,
    activeThreadId: 'thread-1',
  },
  socket: { status: 'connected' },
  inference: {
    sendingByThread: {},
    toolTimelineByThread: {},
    inferenceStatusByThread: {},
    streamingAssistantByThread: {},
  },
  channelConnections: { defaultMessagingChannel: 'web' },
};

vi.mock('react-router-dom', () => ({ useNavigate: () => mockNavigate }));

vi.mock('../../store/hooks', () => ({
  useAppDispatch: () => mockDispatch,
  useAppSelector: (selector: (state: typeof mockState) => unknown) => selector(mockState),
}));

vi.mock('../../store/socketSelectors', () => ({
  selectSocketStatus: (state: typeof mockState) => state.socket.status,
}));

vi.mock('../../store/threadSlice', () => ({
  addMessageLocal: vi.fn(() => ({ type: 'thread/addMessageLocal' })),
  createThreadLocal: vi.fn(() => ({ type: 'thread/createThreadLocal' })),
  fetchSuggestedQuestions: vi.fn(() => ({ type: 'thread/fetchSuggestedQuestions' })),
  loadThreadMessages: (...args: [string]) => mockLoadThreadMessages(...args),
  loadThreads: vi.fn(() => ({ type: 'thread/loadThreads' })),
  persistReaction: vi.fn(() => ({ type: 'thread/persistReaction' })),
  setActiveThread: vi.fn((payload: string | null) => ({ type: 'thread/setActiveThread', payload })),
  setLastViewed: vi.fn((payload: string) => ({ type: 'thread/setLastViewed', payload })),
  setSelectedThread: vi.fn((payload: string) => ({ type: 'thread/setSelectedThread', payload })),
}));

vi.mock('../../hooks/useUsageState', () => ({
  useUsageState: () => ({
    teamUsage: null,
    isLoading: false,
    isAtLimit: false,
    isBudgetExhausted: false,
    isRateLimited: false,
    isNearLimit: false,
    isFreeTier: true,
    shouldShowBudgetCompletedMessage: false,
    usagePct10h: 0,
    usagePct7d: 0,
    currentTier: 'free',
  }),
}));

vi.mock('../../services/chatService', () => ({
  chatCancel: vi.fn(),
  chatSend: vi.fn(),
  useRustChat: () => true,
}));

vi.mock('../../services/chatEventManager', () => ({
  chatEventManager: { setPendingReaction: vi.fn(), clearPendingReaction: vi.fn() },
}));

vi.mock('../../utils/tauriCommands', () => ({
  isTauri: () => false,
  notifyOverlaySttState: vi.fn(),
  openhumanAutocompleteAccept: vi.fn(async () => ({})),
  openhumanAutocompleteCurrent: vi.fn(async () => ({ result: { suggestion: { value: '' } } })),
  openhumanVoiceStatus: vi.fn(async () => ({ stt_available: true })),
  openhumanVoiceTranscribeBytes: vi.fn(async () => ({ text: '' })),
  openhumanVoiceTts: vi.fn(async () => ({ output_path: '' })),
}));

vi.mock('../../components/upsell/UpsellBanner', () => ({ default: () => null }));

vi.mock('../../components/upsell/UsageLimitModal', () => ({ default: () => null }));

vi.mock('../../components/upsell/upsellDismissState', () => ({
  dismissBanner: vi.fn(),
  shouldShowBanner: () => false,
}));

describe('Conversations remount recovery', () => {
  beforeEach(() => {
    mockDispatch.mockClear();
    mockLoadThreadMessages.mockClear();
  });

  it('re-fetches selected thread messages when active thread is still in-flight', async () => {
    const Conversations = (await import('../Conversations')).default;
    render(<Conversations />);

    await waitFor(() => {
      expect(mockLoadThreadMessages).toHaveBeenCalledWith('thread-1');
    });
  });
});
