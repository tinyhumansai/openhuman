/**
 * The thread todo board must render on the surface `/chat` actually shows.
 *
 * `ThreadTodoStrip`'s only mount was inside `legacyMainPanel`, and the panel
 * choice is an either/or - `composer === 'mic-cloud' ? legacyMainPanel :
 * assistantUiMainPanel` - so on the default text composer the strip was
 * unreachable. `onTaskBoardUpdated` kept filling `taskBoardByThread` and
 * nothing downstream ever read it: a long multi-step turn lost its entire
 * plan/progress strip.
 *
 * These tests render the page the way the route does (no `composer` prop) and
 * assert the strip is on it.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

import { SidebarSlotOutlet, SidebarSlotProvider } from '../../components/layout/shell/SidebarSlot';
import agentProfileReducer from '../../store/agentProfileSlice';
import chatRuntimeReducer, { setTaskBoardForThread } from '../../store/chatRuntimeSlice';
import layoutReducer from '../../store/layoutSlice';
import socketReducer from '../../store/socketSlice';
import themeReducer from '../../store/themeSlice';
import threadReducer from '../../store/threadSlice';
import type { Thread } from '../../types/thread';
import type { TaskBoard } from '../../types/turnState';

const { mockGetThreads, mockGetThreadMessages, mockGetTaskBoard, mockUseUsageState } = vi.hoisted(
  () => ({
    mockGetThreads: vi.fn().mockResolvedValue({ threads: [], count: 0 }),
    mockGetThreadMessages: vi.fn().mockResolvedValue({ messages: [], count: 0 }),
    mockGetTaskBoard: vi.fn().mockResolvedValue(null),
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
  })
);

vi.mock('../../services/chatService', () => ({
  chatCancel: vi.fn().mockResolvedValue(true),
  chatClearQueue: vi.fn().mockResolvedValue(0),
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
    getTurnStateHistory: vi.fn().mockResolvedValue([]),
    getDerivedTranscript: vi
      .fn()
      .mockResolvedValue({
        threadId: 'none',
        items: [],
        total: 0,
        hasMore: false,
        hasTranscript: false,
      }),
    getTaskBoard: mockGetTaskBoard,
    putTaskBoard: vi
      .fn()
      .mockResolvedValue({ threadId: 't-1', cards: [], updatedAt: '2026-05-04T10:00:00Z' }),
    decidePlan: vi
      .fn()
      .mockResolvedValue({ threadId: 't-1', cards: [], updatedAt: '2026-05-04T10:00:00Z' }),
    appendMessage: vi.fn(async (_threadId: string, message: unknown) => message),
    deleteThread: vi.fn().mockResolvedValue({ deleted: true }),
    generateTitleIfNeeded: vi.fn().mockResolvedValue({}),
    updateMessage: vi.fn().mockResolvedValue({}),
    purge: vi.fn().mockResolvedValue({}),
    updateLabels: vi.fn().mockResolvedValue({}),
    updateTitle: vi.fn().mockResolvedValue({}),
    persistReaction: vi.fn().mockResolvedValue({}),
    listRuns: vi.fn().mockResolvedValue([]),
    listRunEvents: vi.fn().mockResolvedValue([]),
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

vi.mock('../../services/api/openrouterFreeModels', () => ({
  applyOpenRouterFreeModels: () => undefined,
}));

vi.mock('../../hooks/useUsageState', () => ({ useUsageState: mockUseUsageState }));

vi.mock('../../components/chat/ChatNewWindowHero', () => ({ default: () => null }));

vi.mock('../../store/socketSelectors', () => ({
  selectSocketStatus: (state: { socket?: { byUser?: Record<string, { status: string }> } }) =>
    state.socket?.byUser?.__pending__?.status ?? 'disconnected',
}));

vi.mock('../../hooks/useStickToBottom', () => ({
  useStickToBottom: vi.fn(() => ({ containerRef: { current: null }, endRef: { current: null } })),
}));

vi.mock('../../utils/openUrl', () => ({ openUrl: vi.fn() }));

const mockCallCoreRpc = vi.fn().mockResolvedValue({});
vi.mock('../../services/coreRpcClient', async orig => {
  const actual = await orig<typeof import('../../services/coreRpcClient')>();
  return { ...actual, callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args) };
});

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

const THREAD_ID = 'board-thread';

const thread: Thread = {
  id: THREAD_ID,
  title: 'Board thread',
  chatId: null,
  isActive: false,
  messageCount: 0,
  lastMessageAt: '2026-01-01T00:00:00.000Z',
  createdAt: '2026-01-01T00:00:00.000Z',
  labels: ['general'],
};

const board: TaskBoard = {
  threadId: THREAD_ID,
  updatedAt: '2026-09-03T10:00:00Z',
  cards: [
    {
      id: 'card-1',
      title: 'Read the migration audit',
      status: 'in_progress',
      order: 0,
      updatedAt: '2026-09-03T10:00:00Z',
    },
    {
      id: 'card-2',
      title: 'Re-wire the composer header',
      status: 'todo',
      order: 1,
      updatedAt: '2026-09-03T10:00:00Z',
    },
  ],
};

function buildStore(preload: Record<string, unknown>) {
  return configureStore({
    reducer: combineReducers({
      thread: threadReducer,
      layout: layoutReducer,
      socket: socketReducer,
      chatRuntime: chatRuntimeReducer,
      agentProfiles: agentProfileReducer,
      theme: themeReducer,
    }),
    preloadedState: preload as never,
  });
}

async function renderChat(taskBoard: TaskBoard | null) {
  mockGetThreads.mockResolvedValue({ threads: [thread], count: 1 });
  // The page also hydrates the board from the core on thread select; leaving
  // this at the default would overwrite whatever the socket path put in Redux.
  mockGetTaskBoard.mockResolvedValue(taskBoard);
  const store = buildStore({
    thread: {
      threads: [thread],
      selectedThreadId: THREAD_ID,
      activeThreadIds: {},
      welcomeThreadId: null,
      messagesByThreadId: { [THREAD_ID]: [] },
      messages: [],
      isLoadingThreads: false,
      isLoadingMessages: false,
      messagesError: null,
    },
    socket: { byUser: { __pending__: { status: 'connected', socketId: 'socket-1' } } },
  });
  // Through the real reducer, exactly as `onTaskBoardUpdated` does. A partial
  // `chatRuntime` in `preloadedState` would replace the whole slice and drop
  // every other key the page reads.
  if (taskBoard) store.dispatch(setTaskBoardForThread({ threadId: THREAD_ID, board: taskBoard }));
  const { default: Conversations } = await import('./Conversations');

  await act(async () => {
    render(
      <Provider store={store}>
        <MemoryRouter initialEntries={['/chat']}>
          <SidebarSlotProvider>
            <SidebarSlotOutlet />
            {/* `composer` omitted on purpose: that is what the /chat route
                does, and it is what selects `assistantUiMainPanel`. */}
            <Conversations />
          </SidebarSlotProvider>
        </MemoryRouter>
      </Provider>
    );
  });
  return store;
}

describe('thread task board on the assistant-ui chat surface', () => {
  it('renders the todo strip on the default text composer', async () => {
    await renderChat(board);

    const strip = await screen.findByTestId('thread-todo-strip');
    expect(strip).toBeInTheDocument();
    expect(strip).toHaveTextContent('Read the migration audit');
    expect(strip).toHaveTextContent('Re-wire the composer header');
  });

  it('stays out of the way when the thread has no board', async () => {
    await renderChat(null);
    expect(screen.queryByTestId('thread-todo-strip')).not.toBeInTheDocument();
  });
});
