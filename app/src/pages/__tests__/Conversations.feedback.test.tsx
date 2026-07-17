import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { SidebarSlotOutlet, SidebarSlotProvider } from '../../components/layout/shell/SidebarSlot';
import agentProfileReducer from '../../store/agentProfileSlice';
import chatRuntimeReducer from '../../store/chatRuntimeSlice';
import socketReducer from '../../store/socketSlice';
import threadReducer from '../../store/threadSlice';
import type { Thread, ThreadMessage } from '../../types/thread';

// ── Hoisted mock state ──────────────────────────────────────────────────────

const { mockGetThreads, mockGetThreadMessages, mockUseUsageState } = vi.hoisted(() => ({
  mockGetThreads: vi.fn().mockResolvedValue({ threads: [], count: 0 }),
  mockGetThreadMessages: vi.fn().mockResolvedValue({ messages: [], count: 0 }),
  mockUseUsageState: vi.fn(() => ({
    teamUsage: null,
    currentPlan: null,
    currentTier: 'FREE' as const,
    isFreeTier: true,
    usagePct: 0,
    isNearLimit: false,
    isAtLimit: false,
    isBudgetExhausted: false,
    shouldShowBudgetCompletedMessage: false,
    isLoading: false,
    refresh: vi.fn(),
  })),
}));

// ── Module mocks ────────────────────────────────────────────────────────────

vi.mock('../../services/chatService', () => ({
  chatCancel: vi.fn(),
  chatSend: vi.fn().mockResolvedValue(undefined),
  subscribeChatEvents: vi.fn(() => () => {}),
  useRustChat: vi.fn(() => true),
}));

vi.mock('../../services/api/threadApi', () => ({
  threadApi: {
    createNewThread: vi.fn().mockResolvedValue({ id: 'new-thread', labels: [] }),
    getThreads: mockGetThreads,
    getThreadMessages: mockGetThreadMessages,
    getTurnState: vi.fn().mockResolvedValue(null),
    getTaskBoard: vi
      .fn()
      .mockResolvedValue({ threadId: 't-1', cards: [], updatedAt: '2026-01-01T00:00:00Z' }),
    putTaskBoard: vi
      .fn()
      .mockResolvedValue({ threadId: 't-1', cards: [], updatedAt: '2026-01-01T00:00:00Z' }),
    appendMessage: vi.fn().mockResolvedValue({}),
    deleteThread: vi.fn().mockResolvedValue({ deleted: true }),
    generateTitleIfNeeded: vi.fn().mockResolvedValue({}),
    updateMessage: vi.fn().mockResolvedValue({}),
    purge: vi.fn().mockResolvedValue({}),
    updateLabels: vi.fn().mockResolvedValue({}),
    updateTitle: vi.fn().mockResolvedValue({}),
    persistReaction: vi.fn().mockResolvedValue({}),
  },
}));

vi.mock('../../services/api/agentProfilesApi', () => ({
  agentProfilesApi: {
    list: vi.fn().mockResolvedValue({ activeProfileId: 'default', profiles: [] }),
    select: vi.fn().mockResolvedValue({ activeProfileId: 'default', profiles: [] }),
    upsert: vi.fn().mockResolvedValue({ activeProfileId: 'default', profiles: [] }),
    delete: vi.fn().mockResolvedValue({ activeProfileId: 'default', profiles: [] }),
  },
}));

vi.mock('../../hooks/useUsageState', () => ({ useUsageState: mockUseUsageState }));

vi.mock('../../components/chat/ChatNewWindowHero', () => ({ default: () => null }));

vi.mock('../../store/socketSelectors', () => ({ selectSocketStatus: () => 'connected' }));

vi.mock('../../hooks/useStickToBottom', () => ({
  useStickToBottom: vi.fn(() => ({ containerRef: { current: null }, endRef: { current: null } })),
}));

vi.mock('../../features/autocomplete/useAutocompleteSkillStatus', () => ({
  useAutocompleteSkillStatus: vi.fn(() => ({ status: 'idle', skills: [] })),
}));

vi.mock('../../utils/openUrl', () => ({ openUrl: vi.fn() }));

const mockCallCoreRpc = vi.fn().mockResolvedValue({ ok: true });
vi.mock('../../services/coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
  CoreRpcError: class CoreRpcError extends Error {},
}));

vi.mock('../../lib/coreState/store', () => ({
  getCoreStateSnapshot: vi.fn(() => ({
    isBootstrapping: false,
    isReady: true,
    snapshot: {
      auth: { isAuthenticated: false, userId: null, user: null, profileId: null },
      sessionToken: null,
      currentUser: null,
      onboardingCompleted: true,
      chatOnboardingCompleted: true,
      analyticsEnabled: false,
      localState: {},
      runtime: {},
    },
  })),
  isWelcomeLocked: vi.fn(() => false),
  setCoreStateSnapshot: vi.fn(),
}));

// ── Helpers ─────────────────────────────────────────────────────────────────

function buildStore(preload: Record<string, unknown> = {}) {
  return configureStore({
    reducer: combineReducers({
      thread: threadReducer,
      socket: socketReducer,
      chatRuntime: chatRuntimeReducer,
      agentProfiles: agentProfileReducer,
    }),
    preloadedState: preload as never,
  });
}

function makeThread(overrides: Partial<Thread> = {}): Thread {
  return {
    id: 'feedback-thread',
    title: 'Feedback Test',
    chatId: null,
    isActive: false,
    messageCount: 1,
    lastMessageAt: '2026-01-01T00:01:00.000Z',
    createdAt: '2026-01-01T00:00:00.000Z',
    labels: [],
    ...overrides,
  };
}

function socketState(status: 'connected' | 'disconnected') {
  return {
    byUser: { __pending__: { status, socketId: status === 'connected' ? 'socket-1' : null } },
  };
}

async function renderWithFeedback() {
  const thread = makeThread({ id: 'feedback-thread', title: 'Feedback Thread' });
  const messages: ThreadMessage[] = [
    {
      id: 'backend-trace-123',
      sender: 'agent',
      type: 'text',
      content: 'Here is my response to your question.',
      extraMetadata: {},
      createdAt: '2026-01-01T00:01:00.000Z',
    },
  ];
  mockGetThreads.mockResolvedValue({ threads: [thread], count: 1 });
  mockGetThreadMessages.mockResolvedValue({ messages, count: messages.length });

  const store = buildStore({
    thread: {
      threads: [thread],
      selectedThreadId: thread.id,
      activeThreadIds: {},
      welcomeThreadId: null,
      messagesByThreadId: { [thread.id]: messages },
      messages,
      isLoadingThreads: false,
      isLoadingMessages: false,
      messagesError: null,
    },
    socket: socketState('connected'),
  });

  const { default: Conversations } = await import('../../features/conversations/Conversations');

  await act(async () => {
    render(
      <Provider store={store}>
        <MemoryRouter initialEntries={['/conversations']}>
          <SidebarSlotProvider>
            <SidebarSlotOutlet />
            <Conversations />
          </SidebarSlotProvider>
        </MemoryRouter>
      </Provider>
    );
  });
}

// ── Tests ───────────────────────────────────────────────────────────────────

describe('Conversations — feedback buttons', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockGetThreads.mockResolvedValue({ threads: [], count: 0 });
    mockGetThreadMessages.mockResolvedValue({ messages: [], count: 0 });
    mockUseUsageState.mockReturnValue({
      teamUsage: null,
      currentPlan: null,
      currentTier: 'FREE' as const,
      isFreeTier: true,
      usagePct: 0,
      isNearLimit: false,
      isAtLimit: false,
      isBudgetExhausted: false,
      shouldShowBudgetCompletedMessage: false,
      isLoading: false,
      refresh: vi.fn(),
    });
  });

  it('renders thumbs up and thumbs down on the latest agent message', async () => {
    await renderWithFeedback();

    await waitFor(() => {
      expect(screen.getByTitle('Good response')).toBeInTheDocument();
    });
    const thumbsDown = screen.getByTitle('Bad response');
    expect(thumbsDown).toBeInTheDocument();
  });

  it('calls callCoreRpc with the correct single-object shape for good response', async () => {
    await renderWithFeedback();

    const thumbsUp = await screen.findByTitle('Good response');
    await act(async () => {
      thumbsUp.click();
    });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.observability_submit_score',
      params: { trace_id: 'feedback-thread:backend-trace-123', name: 'user-feedback', value: 1.0 },
    });
  });

  it('calls callCoreRpc with the correct single-object shape for bad response', async () => {
    await renderWithFeedback();

    const thumbsDown = await screen.findByTitle('Bad response');
    await act(async () => {
      thumbsDown.click();
    });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.observability_submit_score',
      params: { trace_id: 'feedback-thread:backend-trace-123', name: 'user-feedback', value: 0.0 },
    });
  });

  it('warns when the good-response score submission fails', async () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    mockCallCoreRpc.mockRejectedValueOnce(new Error('submit failed'));
    await renderWithFeedback();

    const thumbsUp = await screen.findByTitle('Good response');
    await act(async () => {
      thumbsUp.click();
    });

    await waitFor(() =>
      expect(warnSpy).toHaveBeenCalledWith('[feedback] failed to submit good-response score')
    );
    warnSpy.mockRestore();
  });

  it('warns when the bad-response score submission fails', async () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    mockCallCoreRpc.mockRejectedValueOnce(new Error('submit failed'));
    await renderWithFeedback();

    const thumbsDown = await screen.findByTitle('Bad response');
    await act(async () => {
      thumbsDown.click();
    });

    await waitFor(() =>
      expect(warnSpy).toHaveBeenCalledWith('[feedback] failed to submit bad-response score')
    );
    warnSpy.mockRestore();
  });

  it('uses extraMetadata.traceId when available instead of fallback', async () => {
    const thread = makeThread({ id: 'trace-id-thread', title: 'Trace ID Test' });
    const messages: ThreadMessage[] = [
      {
        id: 'msg-1',
        sender: 'agent',
        type: 'text',
        content: 'Message with trace id.',
        extraMetadata: { traceId: 'custom-trace-42' },
        createdAt: '2026-01-01T00:01:00.000Z',
      },
    ];
    mockGetThreads.mockResolvedValue({ threads: [thread], count: 1 });
    mockGetThreadMessages.mockResolvedValue({ messages, count: messages.length });

    const store = buildStore({
      thread: {
        threads: [thread],
        selectedThreadId: thread.id,
        activeThreadIds: {},
        welcomeThreadId: null,
        messagesByThreadId: { [thread.id]: messages },
        messages,
        isLoadingThreads: false,
        isLoadingMessages: false,
        messagesError: null,
      },
      socket: socketState('connected'),
    });

    const { default: Conversations } = await import('../../features/conversations/Conversations');
    await act(async () => {
      render(
        <Provider store={store}>
          <MemoryRouter initialEntries={['/conversations']}>
            <SidebarSlotProvider>
              <SidebarSlotOutlet />
              <Conversations />
            </SidebarSlotProvider>
          </MemoryRouter>
        </Provider>
      );
    });

    const thumbsUp = await screen.findByTitle('Good response');
    await act(async () => {
      thumbsUp.click();
    });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.observability_submit_score',
      params: { trace_id: 'custom-trace-42', name: 'user-feedback', value: 1.0 },
    });
  });

  it('does not render feedback buttons when there are no agent messages', async () => {
    const thread = makeThread({ id: 'empty-thread', title: 'Empty' });
    mockGetThreads.mockResolvedValue({ threads: [thread], count: 1 });
    mockGetThreadMessages.mockResolvedValue({ messages: [], count: 0 });

    const store = buildStore({
      thread: {
        threads: [thread],
        selectedThreadId: thread.id,
        activeThreadIds: {},
        welcomeThreadId: null,
        messagesByThreadId: { [thread.id]: [] },
        messages: [],
        isLoadingThreads: false,
        isLoadingMessages: false,
        messagesError: null,
      },
      socket: socketState('connected'),
    });

    const { default: Conversations } = await import('../../features/conversations/Conversations');
    await act(async () => {
      render(
        <Provider store={store}>
          <MemoryRouter initialEntries={['/conversations']}>
            <SidebarSlotProvider>
              <SidebarSlotOutlet />
              <Conversations />
            </SidebarSlotProvider>
          </MemoryRouter>
        </Provider>
      );
    });

    expect(screen.queryByTitle('Good response')).not.toBeInTheDocument();
    expect(screen.queryByTitle('Bad response')).not.toBeInTheDocument();
  });
});
