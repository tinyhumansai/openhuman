/**
 * Regression test for #3785 — "UI elements hidden when window is too small,
 * obscuring actionable error states".
 *
 * On the Human page the chat embed renders the sidebar variant of
 * Conversations. Its composer footer stacks the upsell/error banners, the
 * actionable error CTAs (e.g. the voice-transcription "Setup" link), and the
 * composer itself in a single block inside the `overflow-hidden` mainPanel.
 * The footer used to be a plain `flex-shrink-0` block, so on a short window its
 * natural height exceeded the panel and its bottom was clipped with no scroll
 * affordance — the composer and the fix button became unreachable.
 *
 * The fix lets the footer SHRINK and scroll instead of staying rigid: dropping
 * `flex-shrink-0` and adding `min-h-0 overflow-y-auto` makes the flex algorithm
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
import agentProfileReducer from '../../store/agentProfileSlice';
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
    getTaskBoard: vi.fn().mockResolvedValue({ threadId: 't-1', cards: [], updatedAt: '' }),
    putTaskBoard: vi.fn().mockResolvedValue({ threadId: 't-1', cards: [], updatedAt: '' }),
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
    list: vi
      .fn()
      .mockResolvedValue({
        activeProfileId: 'default',
        profiles: [
          {
            id: 'default',
            name: 'Default',
            description: 'Default',
            agentId: 'orchestrator',
            builtIn: true,
          },
        ],
      }),
    select: vi.fn().mockResolvedValue({ activeProfileId: 'default', profiles: [] }),
    upsert: vi.fn().mockResolvedValue({ activeProfileId: 'default', profiles: [] }),
    delete: vi.fn().mockResolvedValue({ activeProfileId: 'default', profiles: [] }),
  },
}));

vi.mock('../../services/api/openrouterFreeModels', () => ({ applyOpenRouterFreeModels: vi.fn() }));

vi.mock('../../hooks/useUsageState', () => ({ useUsageState: mockUseUsageState }));

// The new-window hero pulls useUser/useCoreState; stub it so the page renders
// without a CoreStateProvider.
vi.mock('../../components/chat/ChatNewWindowHero', () => ({ default: () => null }));

vi.mock('../../store/socketSelectors', () => ({
  selectSocketStatus: (state: { socket?: { byUser?: Record<string, { status: string }> } }) =>
    state.socket?.byUser?.__pending__?.status ?? 'disconnected',
}));

// useStickToBottom returns refs; mock it so layout-effects don't fire in jsdom.
vi.mock('../../hooks/useStickToBottom', () => ({
  useStickToBottom: vi.fn(() => ({ containerRef: { current: null }, endRef: { current: null } })),
}));

vi.mock('../../features/autocomplete/useAutocompleteSkillStatus', () => ({
  useAutocompleteSkillStatus: vi.fn(() => ({ status: 'idle', skills: [] })),
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

// ── Helpers ────────────────────────────────────────────────────────────────

/** Build a minimal Redux store with the slices Conversations reads, optionally preloaded. */
function buildStore(preload: Record<string, unknown> = {}) {
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

/** Construct a `Thread` fixture with sensible defaults, overridable per field. */
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

/** Thread-slice preload with `thread` present, selected, and holding an empty message list. */
function selectedThreadState(thread: Thread) {
  return {
    ...emptyThreadState,
    threads: [thread],
    selectedThreadId: thread.id,
    messagesByThreadId: { [thread.id]: [] },
    messages: [],
  };
}

/** Socket-slice preload that pins the pending-user connection to the given status. */
function socketState(status: 'connected' | 'disconnected') {
  return {
    byUser: { __pending__: { status, socketId: status === 'connected' ? 'socket-1' : null } },
  };
}

/** Render the Human-page chat embed: sidebar variant with the mic-cloud composer. */
async function renderSidebar(preload: Record<string, unknown> = {}) {
  const store = buildStore(preload);
  const { default: Conversations } = await import('../Conversations');

  let container!: HTMLElement;
  await act(async () => {
    ({ container } = render(
      <Provider store={store}>
        <MemoryRouter initialEntries={['/human']}>
          {/* The thread sidebar is projected into the root sidebar slot, so the
              embed needs a provider + outlet for that portal to mount. */}
          <SidebarSlotProvider>
            <SidebarSlotOutlet />
            <Conversations variant="sidebar" composer="mic-cloud" projectThreadList />
          </SidebarSlotProvider>
        </MemoryRouter>
      </Provider>
    ));
  });

  return { store, container };
}

// ── Tests ──────────────────────────────────────────────────────────────────

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
    // Shrinkable + internally scrollable: on a short window the flex algorithm
    // caps the footer to the available height and it scrolls, instead of being
    // silently clipped by the overflow-hidden mainPanel. (Uses flex shrink, not
    // a percentage max-height — the latter doesn't reliably resolve inside a
    // stretched flex item in Chromium.)
    expect(footer).toHaveClass('overflow-y-auto');
    expect(footer).toHaveClass('min-h-0');
    // It must be allowed to shrink (no flex-shrink-0) so it can give way + scroll.
    expect(footer).not.toHaveClass('flex-shrink-0');
  });

  it('keeps the floating page-variant composer absolutely positioned (no regression)', async () => {
    const store = buildStore({ thread: emptyThreadState });
    const { default: Conversations } = await import('../Conversations');

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

    const footer = container.querySelector('[data-walkthrough="home-cta"]');
    expect(footer).not.toBeNull();
    // Page variant floats over the message fade; it must NOT adopt the sidebar's
    // in-flow scroll cap.
    expect(footer).toHaveClass('absolute');
    expect(footer).not.toHaveClass('overflow-y-auto');
  });
});
