/**
 * Smoke render tests for Conversations.tsx — covers new lines added in #1123
 * (welcome-lock removal: unconditional sidebar, label filter, effectiveShowSidebar,
 * quota usage pills, etc.).
 *
 * These tests intentionally do not test complex user interactions; they verify
 * that the key JSX branches render without crashing, driving coverage of the
 * previously-blocked lines that are now always rendered.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import chatRuntimeReducer from '../../store/chatRuntimeSlice';
import socketReducer from '../../store/socketSlice';
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
      fiveHourCapUsd: number;
      cycleLimit5hr: number;
      bypassCycleLimit: boolean;
      fiveHourResetsAt: string | null;
      cycleEndsAt: string | null;
    },
    currentPlan: null,
    currentTier: 'FREE' as 'FREE' | 'BASIC' | 'PRO',
    isFreeTier: true,
    usagePct10h: 0,
    usagePct7d: 0,
    isNearLimit: false,
    isAtLimit: false,
    isRateLimited: false,
    isBudgetExhausted: false,
    shouldShowBudgetCompletedMessage: false,
    isLoading: false,
    refresh: vi.fn(),
  })),
}));

// ── Module mocks ───────────────────────────────────────────────────────────

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
    appendMessage: vi.fn().mockResolvedValue({}),
    deleteThread: vi.fn().mockResolvedValue({ deleted: true }),
    generateTitleIfNeeded: vi.fn().mockResolvedValue({}),
    updateMessage: vi.fn().mockResolvedValue({}),
    purge: vi.fn().mockResolvedValue({}),
    updateLabels: vi.fn().mockResolvedValue({}),
    persistReaction: vi.fn().mockResolvedValue({}),
  },
}));

vi.mock('../../hooks/useUsageState', () => ({ useUsageState: mockUseUsageState }));

// useStickToBottom returns refs; mock it so layout-effects don't fire in jsdom.
vi.mock('../../hooks/useStickToBottom', () => ({
  useStickToBottom: vi.fn(() => ({ containerRef: { current: null }, endRef: { current: null } })),
}));

// useAutocompleteSkillStatus may make API calls; stub it.
vi.mock('../../features/autocomplete/useAutocompleteSkillStatus', () => ({
  useAutocompleteSkillStatus: vi.fn(() => ({ status: 'idle', skills: [] })),
}));

// openUrl uses Tauri; stub it.
vi.mock('../../utils/openUrl', () => ({ openUrl: vi.fn() }));

// coreState/store: getCoreStateSnapshot used by selectSocketStatus.
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

function buildStore(preload: Record<string, unknown> = {}) {
  return configureStore({
    reducer: combineReducers({
      thread: threadReducer,
      socket: socketReducer,
      chatRuntime: chatRuntimeReducer,
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
    labels: [],
    ...overrides,
  };
}

async function renderConversations(preload: Record<string, unknown> = {}) {
  const store = buildStore(preload);
  const { default: Conversations } = await import('../Conversations');

  render(
    <Provider store={store}>
      <MemoryRouter initialEntries={['/conversations']}>
        <Conversations />
      </MemoryRouter>
    </Provider>
  );

  return store;
}

// Default empty state
const emptyThreadState = {
  threads: [],
  selectedThreadId: null,
  activeThreadId: null,
  welcomeThreadId: null,
  messagesByThreadId: {},
  messages: [],
  isLoadingThreads: false,
  isLoadingMessages: false,
  messagesError: null,
};

// ── Tests ──────────────────────────────────────────────────────────────────

describe('Conversations — smoke render (#1123 welcome-lock removal)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // Reset the mock to defaults for each test
    mockGetThreads.mockResolvedValue({ threads: [], count: 0 });
    mockGetThreadMessages.mockResolvedValue({ messages: [], count: 0 });
    mockUseUsageState.mockReturnValue({
      teamUsage: null,
      currentPlan: null,
      currentTier: 'FREE' as const,
      isFreeTier: true,
      usagePct10h: 0,
      usagePct7d: 0,
      isNearLimit: false,
      isAtLimit: false,
      isRateLimited: false,
      isBudgetExhausted: false,
      shouldShowBudgetCompletedMessage: false,
      isLoading: false,
      refresh: vi.fn(),
    });
  });

  // Covers line 906: const effectiveShowSidebar = showSidebar;
  // Covers line 941: <div className="flex-1 overflow-y-auto"> (always rendered in page mode)
  it('renders the Threads sidebar header in page mode', async () => {
    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    // The "Threads" header is always rendered in page mode (sidebar guard removed)
    expect(screen.getByText('Threads')).toBeInTheDocument();
  });

  // Covers line 941 empty branch
  it('shows "No threads yet" when thread list is empty', async () => {
    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    expect(screen.getByText('No threads yet')).toBeInTheDocument();
  });

  // Covers lines 1002-1004, 1007, 1011-1012, 1014: thread list items rendered unconditionally
  it('renders thread list items when threads are pre-loaded', async () => {
    const threads = [
      makeThread({ id: 't-1', title: 'Thread Alpha' }),
      makeThread({ id: 't-2', title: 'Thread Beta' }),
    ];

    // Return the threads from the API so the useEffect loadThreads picks them up
    mockGetThreads.mockResolvedValue({ threads, count: 2 });

    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    // Wait for loadThreads to complete and the thread list to render.
    // Use getAllByText because the title may appear in both the sidebar list
    // and the conversation header (both are rendered).
    await waitFor(() => {
      expect(screen.getAllByText('Thread Alpha').length).toBeGreaterThan(0);
    });
    expect(screen.getAllByText('Thread Beta').length).toBeGreaterThan(0);
  });

  // Covers line 1083: messagesError branch renders error state
  it('renders the error icon section when loadThreadMessages rejects', async () => {
    // Make loadThreadMessages always fail so messagesError is set in the store
    mockGetThreadMessages.mockRejectedValue(new Error('Network error'));

    // Return one thread so the component selects it and loads messages
    const thread = makeThread({ id: 't-2', title: 'Error Thread' });
    mockGetThreads.mockResolvedValue({ threads: [thread], count: 1 });

    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    // After the failed load, messagesError is set in state — the error branch renders.
    // This covers line 1083 (the error container div).
    await waitFor(() => {
      // The error branch renders "Failed to load messages" static text
      expect(screen.getByText('Failed to load messages')).toBeInTheDocument();
    });
  });

  // Covers lines 1455-1483: quota pill loading state
  it('renders "loading…" quota pill when isLoadingBudget=true', async () => {
    mockUseUsageState.mockReturnValue({
      teamUsage: null,
      currentPlan: null,
      currentTier: 'FREE' as const,
      isFreeTier: true,
      usagePct10h: 0,
      usagePct7d: 0,
      isNearLimit: false,
      isAtLimit: false,
      isRateLimited: false,
      isBudgetExhausted: false,
      shouldShowBudgetCompletedMessage: false,
      isLoading: true,
      refresh: vi.fn(),
    });

    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    expect(screen.getByText('loading…')).toBeInTheDocument();
  });

  // Covers lines 1417-1439: budget banner + lines 1455-1516: LimitPill + tooltip
  it('renders budget-limit banner and limit pills when teamUsage is present', async () => {
    // cycleBudgetUsd: 0 → renders "Your included budget is complete" branch
    const teamUsage = {
      cycleBudgetUsd: 0,
      remainingUsd: 0,
      fiveHourCapUsd: 5,
      cycleLimit5hr: 5,
      bypassCycleLimit: false,
      fiveHourResetsAt: null,
      cycleEndsAt: null,
    };

    mockUseUsageState.mockReturnValue({
      teamUsage,
      currentPlan: null,
      currentTier: 'PRO' as const,
      isFreeTier: false,
      usagePct10h: 1.0,
      usagePct7d: 1.0,
      isNearLimit: true,
      isAtLimit: true,
      isRateLimited: false,
      isBudgetExhausted: true,
      shouldShowBudgetCompletedMessage: true,
      isLoading: false,
      refresh: vi.fn(),
    });

    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    // Budget-exceeded banner (lines 1417-1439) — cycleBudgetUsd=0 gives "included budget" message
    expect(screen.getByText(/Your included budget is complete/i)).toBeInTheDocument();

    // LimitPill components (lines 1459-1480) — their label text
    expect(screen.getByText('7d')).toBeInTheDocument();
  });

  // Covers line 247: if (cancelled) return — the non-cancelled path through loadThreads callback
  it('selects first thread after loadThreads resolves (non-cancelled path)', async () => {
    const threads = [makeThread({ id: 't-1', title: 'First Thread' })];
    mockGetThreads.mockResolvedValue({ threads, count: 1 });

    let resolvedStore: ReturnType<typeof buildStore> | undefined;
    await act(async () => {
      resolvedStore = await renderConversations({ thread: emptyThreadState });
    });

    // After loadThreads resolves and cancelled=false, the first thread is selected.
    // This exercises line 247 (the if (cancelled) return check runs and is false).
    await waitFor(() => {
      const state = resolvedStore?.getState() as { thread: { selectedThreadId: string | null } };
      expect(state.thread.selectedThreadId).toBe('t-1');
    });
  });

  // Covers line 919: onClick={() => void handleCreateNewThread()} — sidebar "New thread" button
  // Covers line 1061: onClick={() => void handleCreateNewThread()} — header "+ New" button
  it('clicking "New thread" sidebar button calls handleCreateNewThread', async () => {
    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    // The sidebar "New thread" button has title="New thread"
    const newThreadBtn = screen.getByTitle('New thread');
    await act(async () => {
      fireEvent.click(newThreadBtn);
    });

    // createNewThread was called — verifies line 919 callback executed
    const { threadApi } = await import('../../services/api/threadApi');
    expect(threadApi.createNewThread).toHaveBeenCalled();
  });

  it('clicking "+ New" header button calls handleCreateNewThread', async () => {
    // Need a selected thread so the header renders
    const threads = [makeThread({ id: 't-1', title: 'Header Thread' })];
    mockGetThreads.mockResolvedValue({ threads, count: 1 });

    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    // Wait for thread to be selected so the header with "+ New" button renders
    await waitFor(() => {
      expect(screen.getByTitle('New thread (/new)')).toBeInTheDocument();
    });

    const headerNewBtn = screen.getByTitle('New thread (/new)');
    await act(async () => {
      fireEvent.click(headerNewBtn);
    });

    // createNewThread was called — verifies line 1061 callback executed
    const { threadApi } = await import('../../services/api/threadApi');
    expect(threadApi.createNewThread).toHaveBeenCalled();
  });

  // Covers lines 981, 982: e.stopPropagation() and setDeleteModal(...) inside delete onClick
  it('clicking delete button on a thread opens the delete modal', async () => {
    const threads = [makeThread({ id: 't-del', title: 'Deletable Thread' })];
    mockGetThreads.mockResolvedValue({ threads, count: 1 });

    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    // Wait for the thread to appear in the sidebar
    await waitFor(() => {
      expect(screen.getAllByText('Deletable Thread').length).toBeGreaterThan(0);
    });

    // The delete button has title="Delete thread"
    const deleteBtn = screen.getByTitle('Delete thread');
    await act(async () => {
      fireEvent.click(deleteBtn);
    });

    // The modal should now be open — "Are you sure you want to delete" text
    // This verifies lines 981, 982, 985 inside the delete onClick callback executed
    expect(screen.getByText(/Are you sure you want to delete/i)).toBeInTheDocument();
  });

  // Covers lines 1399, 1409-1410: isNearLimit UpsellBanner render + onCtaClick
  it('renders near-limit UpsellBanner and clicking Upgrade calls openUrl', async () => {
    const { openUrl } = await import('../../utils/openUrl');

    mockUseUsageState.mockReturnValue({
      teamUsage: null,
      currentPlan: null,
      currentTier: 'FREE' as const,
      isFreeTier: true,
      usagePct10h: 0.85,
      usagePct7d: 0.85,
      isNearLimit: true,
      isAtLimit: false,
      isRateLimited: false,
      isBudgetExhausted: false,
      shouldShowBudgetCompletedMessage: false,
      isLoading: false,
      refresh: vi.fn(),
    });

    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    // UpsellBanner renders with "Approaching usage limit" (line 1399 branch)
    expect(screen.getByText('Approaching usage limit')).toBeInTheDocument();

    // Click the "Upgrade" button — covers line 1409-1410 (onCtaClick callback)
    const upgradeBtn = screen.getByText('Upgrade');
    await act(async () => {
      fireEvent.click(upgradeBtn);
    });

    expect(openUrl).toHaveBeenCalled();
  });

  // Covers line 1413: onDismiss callback inside UpsellBanner
  it('dismissing the near-limit UpsellBanner writes to localStorage (onDismiss executes)', async () => {
    mockUseUsageState.mockReturnValue({
      teamUsage: null,
      currentPlan: null,
      currentTier: 'FREE' as const,
      isFreeTier: true,
      usagePct10h: 0.9,
      usagePct7d: 0.9,
      isNearLimit: true,
      isAtLimit: false,
      isRateLimited: false,
      isBudgetExhausted: false,
      shouldShowBudgetCompletedMessage: false,
      isLoading: false,
      refresh: vi.fn(),
    });

    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    // UpsellBanner renders
    expect(screen.getByText('Approaching usage limit')).toBeInTheDocument();

    // Click dismiss button (aria-label="Dismiss") — covers line 1413 (onDismiss callback)
    const dismissBtn = screen.getByRole('button', { name: 'Dismiss' });
    await act(async () => {
      fireEvent.click(dismissBtn);
    });

    // dismissBanner writes to localStorage with the banner key — confirms line 1413 executed
    expect(localStorage.getItem('openhuman:upsell:conversations-warning')).not.toBeNull();
  });

  // Covers line 1443: onClick inside "Top Up" button in budget-exceeded banner
  it('clicking "Top Up" in the budget banner calls openUrl', async () => {
    const { openUrl } = await import('../../utils/openUrl');

    const teamUsage = {
      cycleBudgetUsd: 10,
      remainingUsd: 0,
      fiveHourCapUsd: 5,
      cycleLimit5hr: 5,
      bypassCycleLimit: false,
      fiveHourResetsAt: null,
      cycleEndsAt: null,
    };

    mockUseUsageState.mockReturnValue({
      teamUsage,
      currentPlan: null,
      currentTier: 'PRO' as const,
      isFreeTier: false,
      usagePct10h: 1.0,
      usagePct7d: 1.0,
      isNearLimit: true,
      isAtLimit: true,
      isRateLimited: false,
      isBudgetExhausted: true,
      shouldShowBudgetCompletedMessage: true,
      isLoading: false,
      refresh: vi.fn(),
    });

    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    // Budget banner renders — cycleBudgetUsd: 10 > 0 → "You've hit your weekly limit"
    expect(screen.getByText(/You've hit your weekly limit/i)).toBeInTheDocument();

    // Click "Top Up" button — covers line 1442-1443 (onClick callback)
    const topUpBtn = screen.getByText('Top Up');
    await act(async () => {
      fireEvent.click(topUpBtn);
    });

    expect(openUrl).toHaveBeenCalled();
  });

  // Covers line 1437: rate-limit message branch (isRateLimited=true, shouldShowBudgetCompletedMessage=false)
  it('renders rate-limit message in budget banner when isRateLimited=true', async () => {
    const teamUsage = {
      cycleBudgetUsd: 10,
      remainingUsd: 5,
      fiveHourCapUsd: 5,
      cycleLimit5hr: 5,
      bypassCycleLimit: false,
      fiveHourResetsAt: null,
      cycleEndsAt: null,
    };

    mockUseUsageState.mockReturnValue({
      teamUsage,
      currentPlan: null,
      currentTier: 'PRO' as const,
      isFreeTier: false,
      usagePct10h: 1.0,
      usagePct7d: 0.5,
      isNearLimit: true,
      isAtLimit: false,
      isRateLimited: true,
      isBudgetExhausted: false,
      shouldShowBudgetCompletedMessage: false,
      isLoading: false,
      refresh: vi.fn(),
    });

    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    // isRateLimited=true, shouldShowBudgetCompletedMessage=false → rate-limit branch (line 1437)
    expect(screen.getByText(/10-hour rate limit reached/i)).toBeInTheDocument();
  });
});
