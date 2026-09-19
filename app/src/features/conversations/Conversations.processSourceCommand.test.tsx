/**
 * The agent-process-source command must not be offered in voice mode.
 *
 * `showProcessSource` only drives `TranscriptOverlays`, and `TranscriptOverlays`
 * mounts inside `assistantUiMainPanel` alone. The panel choice is an either/or -
 * `composer === 'mic-cloud' ? legacyMainPanel : assistantUiMainPanel` - so in
 * mic-cloud (voice/mascot) mode `legacyMainPanel` mounts instead and the state
 * the command sets has no host. Registered with `enabled: () =>
 * selectedThreadId !== null` alone, the palette listed a command that looked
 * available and silently did nothing.
 *
 * Same root cause as `Conversations.taskBoard.test.tsx` next door: something was
 * left pointing at the wrong half of the either/or.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, cleanup, render } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { SidebarSlotOutlet, SidebarSlotProvider } from '../../components/layout/shell/SidebarSlot';
import { registry } from '../../lib/commands/registry';
import chatRuntimeReducer from '../../store/chatRuntimeSlice';
import layoutReducer from '../../store/layoutSlice';
import socketReducer from '../../store/socketSlice';
import themeReducer from '../../store/themeSlice';
import threadReducer from '../../store/threadSlice';
import type { Thread } from '../../types/thread';

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

vi.mock('../../hooks/useUsageState', () => ({ useUsageState: mockUseUsageState }));

vi.mock('../../components/chat/ChatNewWindowHero', () => ({ default: () => null }));

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

const THREAD_ID = 'process-source-thread';

const thread: Thread = {
  id: THREAD_ID,
  title: 'Process source thread',
  chatId: null,
  isActive: false,
  messageCount: 0,
  lastMessageAt: '2026-01-01T00:00:00.000Z',
  createdAt: '2026-01-01T00:00:00.000Z',
  labels: ['general'],
};

const ACTION_ID = 'chat.agentProcessSource';

function buildStore(preload: Record<string, unknown>) {
  return configureStore({
    reducer: combineReducers({
      thread: threadReducer,
      layout: layoutReducer,
      socket: socketReducer,
      chatRuntime: chatRuntimeReducer,
      theme: themeReducer,
    }),
    preloadedState: preload as never,
  });
}

async function renderChat(composer?: 'text' | 'mic-cloud') {
  mockGetThreads.mockResolvedValue({ threads: [thread], count: 1 });
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
  const { default: Conversations } = await import('./Conversations');

  await act(async () => {
    render(
      <Provider store={store}>
        <MemoryRouter initialEntries={['/chat']}>
          <SidebarSlotProvider>
            <SidebarSlotOutlet />
            <Conversations composer={composer} />
          </SidebarSlotProvider>
        </MemoryRouter>
      </Provider>
    );
  });
}

// The predicate's other half (`selectedThreadId !== null`) is deliberately not
// asserted here: on `/chat` it is unreachable as a steady state. The boot effect
// reuses an empty thread or calls `handleCreateNewThread`, so the page always
// ends up with a selection and a test for it would only be pinning the mock.
describe('the agent-process-source command follows the panel that hosts it', () => {
  afterEach(() => {
    cleanup();
    registry.reset();
  });

  it('is enabled on the assistant-ui surface, which mounts TranscriptOverlays', async () => {
    await renderChat('text');

    const action = registry.getAction(ACTION_ID);
    expect(action, 'the command must be registered on the text composer').toBeDefined();
    expect(action?.enabled?.()).toBe(true);
    // The palette runs it through `runAction`, which re-checks `enabled`.
    expect(registry.runAction(ACTION_ID)).toBe(true);
  });

  it('is disabled in mic-cloud voice mode, where nothing renders the panel', async () => {
    await renderChat('mic-cloud');

    const action = registry.getAction(ACTION_ID);
    // Registered but refused - `Conversations` is mounted either way, so the
    // action does not disappear; it must report itself unavailable.
    expect(action, 'the command is still registered in voice mode').toBeDefined();
    expect(action?.enabled?.()).toBe(false);
    expect(registry.runAction(ACTION_ID)).toBe(false);
  });
});
