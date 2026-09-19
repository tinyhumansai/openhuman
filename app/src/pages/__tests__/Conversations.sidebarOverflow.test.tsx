/**
 * Regression test for #3785 — "UI elements hidden when window is too small,
 * obscuring actionable error states".
 *
 * On the Human page the chat embed renders the sidebar variant of
 * Conversations. Its composer footer stacks the upsell/error banners, the
 * actionable error CTAs (e.g. the voice-transcription "Setup" link), and the
 * composer itself in a single block inside the `overflow-hidden` mainPanel.
 * The footer used to be a plain `shrink-0` block, so on a short window its
 * natural height exceeded the panel and its bottom was clipped with no scroll
 * affordance — the composer and the fix button became unreachable.
 *
 * The fix lets the footer SHRINK and scroll instead of staying rigid: dropping
 * `shrink-0` and adding `min-h-0 overflow-y-auto` makes the flex algorithm
 * cap it to the available height and scroll it internally. jsdom does not lay
 * out, so we assert the footer is class-wise scroll-capable + shrinkable, which
 * is what prevents the silent clipping from coming back.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, render } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { SidebarSlotOutlet, SidebarSlotProvider } from '../../components/layout/shell/SidebarSlot';
import chatRuntimeReducer from '../../store/chatRuntimeSlice';
import layoutReducer from '../../store/layoutSlice';
import socketReducer from '../../store/socketSlice';
import themeReducer from '../../store/themeSlice';
import threadReducer from '../../store/threadSlice';
import type { Thread } from '../../types/thread';

// ── Hoisted mock state ─────────────────────────────────────────────────────

const { mockGetThreads, mockGetThreadMessages, mockUseUsageState } = vi.hoisted(() => ({
  mockGetThreads: vi.fn().mockResolvedValue({ threads: [], count: 0 }),
  mockGetThreadMessages: vi.fn().mockResolvedValue({ messages: [], count: 0 }),
  mockUseUsageState: vi.fn(() => ({
    teamUsage: null as null | {
      cycleBudgetUsd: number;
      remainingUsd: number;
      cycleSpentUsd: number;
      cycleEndsAt: string | null;
    },
    currentPlan: null,
    currentTier: 'FREE' as 'FREE' | 'BASIC' | 'PRO',
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

// ── Module mocks (mirror Conversations.render.test.tsx's known-good set) ────

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

vi.mock('../../services/api/openrouterFreeModels', () => ({ applyOpenRouterFreeModels: vi.fn() }));

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

function buildStore(preload: Record<string, unknown> = {}) {
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

function makeThread(overrides: Partial<Thread> = {}): Thread {
  return {
    id: 't-1',
    title: 'Test thread',
    chatId: null,
    isActive: false,
    messageCount: 0,
    lastMessageAt: '2026-01-01T00:00:00.000Z',
    createdAt: '2026-01-01T00:00:00.000Z',
    labels: ['general'],
    ...overrides,
  };
}

const emptyThreadState = {
  threads: [],
  selectedThreadId: null,
  activeThreadIds: {},
  welcomeThreadId: null,
  messagesByThreadId: {},
  messages: [],
  isLoadingThreads: false,
  isLoadingMessages: false,
  messagesError: null,
};

function selectedThreadState(thread: Thread) {
  return {
    ...emptyThreadState,
    threads: [thread],
    selectedThreadId: thread.id,
    messagesByThreadId: { [thread.id]: [] },
    messages: [],
  };
}

function socketState(status: 'connected' | 'disconnected') {
  return {
    byUser: { __pending__: { status, socketId: status === 'connected' ? 'socket-1' : null } },
  };
}

async function renderSidebar(preload: Record<string, unknown> = {}) {
  const store = buildStore(preload);
  const { default: Conversations } = await import('../../features/conversations/Conversations');

  let container!: HTMLElement;
  await act(async () => {
    ({ container } = render(
      <Provider store={store}>
        <MemoryRouter initialEntries={['/human']}>
          <SidebarSlotProvider>
            <SidebarSlotOutlet />
            <Conversations variant="sidebar" composer="mic-cloud" projectThreadList />
          </SidebarSlotProvider>
        </MemoryRouter>
      </Provider>
    ));
  });

  return { container };
}

describe('Conversations — sidebar composer footer overflow (#3785)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    mockGetThreads.mockResolvedValue({ threads: [], count: 0 });
    mockGetThreadMessages.mockResolvedValue({ messages: [], count: 0 });
  });

  it('caps the footer to the panel and scrolls it internally so it cannot be clipped', async () => {
    const thread = makeThread({ id: 'human-thread', title: 'Human' });
    mockGetThreads.mockResolvedValue({ threads: [thread], count: 1 });

    const { container } = await renderSidebar({
      thread: selectedThreadState(thread),
      socket: socketState('connected'),
    });

    const footer = container.querySelector('[data-walkthrough="home-cta"]');
    expect(footer).not.toBeNull();
    expect(footer).toHaveClass('overflow-y-auto');
    expect(footer).toHaveClass('min-h-0');
    expect(footer).not.toHaveClass('shrink-0');
  });

  it('keeps the assistant-ui page composer in flow (no legacy floating footer)', async () => {
    const store = buildStore({ thread: emptyThreadState });
    const { default: Conversations } = await import('../../features/conversations/Conversations');

    let container!: HTMLElement;
    await act(async () => {
      ({ container } = render(
        <Provider store={store}>
          <MemoryRouter initialEntries={['/conversations']}>
            <SidebarSlotProvider>
              <SidebarSlotOutlet />
              <Conversations variant="page" />
            </SidebarSlotProvider>
          </MemoryRouter>
        </Provider>
      ));
    });

    const composer = container.querySelector('[data-slot="aui_composer-shell"]');
    expect(composer).not.toBeNull();
    expect(composer?.closest('.aui-composer-root')).not.toHaveClass('absolute');
    expect(container.querySelector('[data-walkthrough="home-cta"]')).toBeNull();
  });
});
