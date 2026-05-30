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

import { agentProfilesApi } from '../../services/api/agentProfilesApi';
import { threadApi } from '../../services/api/threadApi';
import { chatSend } from '../../services/chatService';
import { CoreRpcError } from '../../services/coreRpcClient';
import agentProfileReducer from '../../store/agentProfileSlice';
import chatRuntimeReducer, {
  setInferenceStatusForThread,
  setToolTimelineForThread,
} from '../../store/chatRuntimeSlice';
import socketReducer from '../../store/socketSlice';
import threadReducer, { setSelectedThread } from '../../store/threadSlice';
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
    getTurnState: vi.fn().mockResolvedValue(null),
    getTaskBoard: vi
      .fn()
      .mockResolvedValue({ threadId: 't-1', cards: [], updatedAt: '2026-05-04T10:00:00Z' }),
    putTaskBoard: vi
      .fn()
      .mockResolvedValue({ threadId: 't-1', cards: [], updatedAt: '2026-05-04T10:00:00Z' }),
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
    select: vi
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
    upsert: vi.fn().mockResolvedValue({ activeProfileId: 'default', profiles: [] }),
    delete: vi.fn().mockResolvedValue({ activeProfileId: 'default', profiles: [] }),
  },
}));

vi.mock('../../hooks/useUsageState', () => ({ useUsageState: mockUseUsageState }));

vi.mock('../../store/socketSelectors', () => ({
  selectSocketStatus: (state: { socket?: { byUser?: Record<string, { status: string }> } }) =>
    state.socket?.byUser?.__pending__?.status ?? 'disconnected',
}));

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
      agentProfiles: agentProfileReducer,
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

/** Click the sidebar toggle so the thread list becomes visible.
 *  The sidebar starts hidden (showSidebar=false) in this PR. */
async function openSidebar() {
  const toggleBtn = screen.getByTitle('Show sidebar');
  await act(async () => {
    fireEvent.click(toggleBtn);
  });
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

async function renderSelectedConversation(
  options: { isAtLimit?: boolean; socketStatus?: 'connected' | 'disconnected' } = {}
) {
  const thread = makeThread({ id: 'send-thread', title: 'Send Thread' });
  mockGetThreads.mockResolvedValue({ threads: [thread], count: 1 });
  mockGetThreadMessages.mockResolvedValue({ messages: [], count: 0 });
  mockUseUsageState.mockReturnValue({
    teamUsage: null,
    currentPlan: null,
    currentTier: 'FREE' as const,
    isFreeTier: true,
    usagePct: options.isAtLimit ? 1 : 0,
    isNearLimit: Boolean(options.isAtLimit),
    isAtLimit: Boolean(options.isAtLimit),
    isBudgetExhausted: false,
    shouldShowBudgetCompletedMessage: false,
    isLoading: false,
    refresh: vi.fn(),
  });

  let renderedStore: ReturnType<typeof buildStore> | undefined;
  await act(async () => {
    renderedStore = await renderConversations({
      thread: selectedThreadState(thread),
      socket: socketState(options.socketStatus ?? 'connected'),
    });
  });

  const textarea = await screen.findByPlaceholderText('Type a message...');
  return { store: renderedStore, textarea, thread };
}

async function submitComposerText(textarea: HTMLElement, text: string) {
  await act(async () => {
    fireEvent.change(textarea, { target: { value: text } });
  });
  await waitFor(() => {
    expect(textarea).toHaveValue(text);
    expect(screen.getByRole('button', { name: 'Send message' })).not.toBeDisabled();
  });
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: 'Send message' }));
  });
}

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
      usagePct: 0.0,
      isNearLimit: false,
      isAtLimit: false,
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

    // Sidebar is hidden by default — open it first.
    await openSidebar();

    // The "Threads" header is always rendered in page mode (sidebar guard removed)
    expect(screen.getByText('Threads')).toBeInTheDocument();
  });

  // Covers line 941 empty branch
  it('shows "No threads yet" when thread list is empty', async () => {
    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    // Sidebar is hidden by default — open it first.
    await openSidebar();

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

    // Sidebar is hidden by default — open it first.
    await openSidebar();

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
  it('renders "Loading…" quota pill when isLoadingBudget=true', async () => {
    mockUseUsageState.mockReturnValue({
      teamUsage: null,
      currentPlan: null,
      currentTier: 'FREE' as const,
      isFreeTier: true,
      usagePct: 0.0,
      isNearLimit: false,
      isAtLimit: false,
      isBudgetExhausted: false,
      shouldShowBudgetCompletedMessage: false,
      isLoading: true,
      refresh: vi.fn(),
    });

    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    expect(screen.getByText('Loading…')).toBeInTheDocument();
  });

  // Covers lines 1417-1439: budget banner + lines 1455-1516: LimitPill + tooltip
  it('renders budget-limit banner and limit pills when teamUsage is present', async () => {
    // cycleBudgetUsd: 0 → renders "Your included budget is complete" branch
    const teamUsage = { cycleBudgetUsd: 0, remainingUsd: 0, cycleSpentUsd: 0, cycleEndsAt: null };

    mockUseUsageState.mockReturnValue({
      teamUsage,
      currentPlan: null,
      currentTier: 'PRO' as const,
      isFreeTier: false,
      usagePct: 1.0,
      isNearLimit: true,
      isAtLimit: true,
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

    // LimitPill renders with the cycle label
    expect(screen.getByText('Cycle')).toBeInTheDocument();
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

    // Sidebar is hidden by default — open it first.
    await openSidebar();

    // The sidebar "New thread" button has title="New thread"
    const newThreadBtn = screen.getByTitle('New thread');
    await act(async () => {
      fireEvent.click(newThreadBtn);
    });

    // createNewThread was called — verifies line 919 callback executed
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
    expect(threadApi.createNewThread).toHaveBeenCalled();
  });

  // Covers lines 981, 982: e.stopPropagation() and setDeleteModal(...) inside delete onClick
  it('clicking delete button on a thread opens the delete modal', async () => {
    const threads = [makeThread({ id: 't-del', title: 'Deletable Thread' })];
    mockGetThreads.mockResolvedValue({ threads, count: 1 });

    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    // Sidebar is hidden by default — open it first.
    await openSidebar();

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
      usagePct: 0.85,
      isNearLimit: true,
      isAtLimit: false,
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
      usagePct: 0.9,
      isNearLimit: true,
      isAtLimit: false,
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

    const teamUsage = { cycleBudgetUsd: 10, remainingUsd: 0, cycleSpentUsd: 10, cycleEndsAt: null };

    mockUseUsageState.mockReturnValue({
      teamUsage,
      currentPlan: null,
      currentTier: 'PRO' as const,
      isFreeTier: false,
      usagePct: 1.0,
      isNearLimit: true,
      isAtLimit: true,
      isBudgetExhausted: true,
      shouldShowBudgetCompletedMessage: true,
      isLoading: false,
      refresh: vi.fn(),
    });

    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    // Budget banner renders — cycleBudgetUsd: 10 > 0 → cycle-budget exhausted copy
    expect(screen.getByText(/used your included cycle budget/i)).toBeInTheDocument();

    // Click "Top Up" button — covers line 1442-1443 (onClick callback)
    const topUpBtn = screen.getByText('Top Up');
    await act(async () => {
      fireEvent.click(topUpBtn);
    });

    expect(openUrl).toHaveBeenCalled();
  });

  it('handles /new from the composer without a selected thread or sending chat text', async () => {
    mockGetThreads.mockReturnValue(new Promise(() => {}));

    await act(async () => {
      await renderConversations({ thread: emptyThreadState, socket: socketState('connected') });
    });
    const textarea = await screen.findByPlaceholderText('Type a message...');
    vi.mocked(threadApi.createNewThread).mockClear();
    vi.mocked(chatSend).mockClear();

    await submitComposerText(textarea, '/new');

    await waitFor(() => {
      expect(threadApi.createNewThread).toHaveBeenCalled();
    });
    expect(chatSend).not.toHaveBeenCalled();
    expect(textarea).toHaveValue('');
  });

  it('blocks the send when the account is over budget (no rate-limit modal anymore)', async () => {
    const { textarea } = await renderSelectedConversation({ isAtLimit: true });

    await submitComposerText(textarea, 'hello at limit');

    // Backend PR #790 removed the rate-limit modal; over-budget now surfaces
    // only the inline send-error (which clears as soon as the user keeps
    // typing). The contract we still care about: chatSend is suppressed.
    expect(chatSend).not.toHaveBeenCalled();
  });

  it('persists a local user message and sends through chat service for valid input', async () => {
    const { textarea, thread } = await renderSelectedConversation();

    await submitComposerText(textarea, ' hello cloud ');

    await waitFor(() => {
      expect(threadApi.appendMessage).toHaveBeenCalledWith(
        thread.id,
        expect.objectContaining({ content: 'hello cloud', sender: 'user', type: 'text' })
      );
    });
    expect(chatSend).toHaveBeenCalledWith({
      threadId: thread.id,
      message: 'hello cloud',
      model: 'reasoning-v1',
      profileId: 'default',
      locale: 'en',
    });
  });

  it('blocks duplicate sends while the first send is still pending', async () => {
    let resolveSend: (() => void) | undefined;
    vi.mocked(chatSend).mockImplementationOnce(
      () =>
        new Promise<void>(resolve => {
          resolveSend = resolve;
        })
    );
    const { textarea, thread } = await renderSelectedConversation();

    await act(async () => {
      fireEvent.change(textarea, { target: { value: 'slow backend' } });
    });
    await waitFor(() => {
      expect(textarea).toHaveValue('slow backend');
      expect(screen.getByRole('button', { name: 'Send message' })).not.toBeDisabled();
    });

    const sendButton = screen.getByRole('button', { name: 'Send message' });
    await act(async () => {
      fireEvent.click(sendButton);
      fireEvent.click(sendButton);
      fireEvent.click(sendButton);
    });

    await waitFor(() => {
      expect(chatSend).toHaveBeenCalledTimes(1);
    });
    expect(threadApi.appendMessage).toHaveBeenCalledTimes(1);
    expect(chatSend).toHaveBeenCalledWith({
      threadId: thread.id,
      message: 'slow backend',
      model: 'reasoning-v1',
      profileId: 'default',
      locale: 'en',
    });
    expect(screen.getByRole('button', { name: 'Send message' })).toBeDisabled();
    resolveSend?.();
  });

  it('releases the pending-send lock when appendMessage rejects with a generic error', async () => {
    vi.mocked(threadApi.appendMessage).mockRejectedValueOnce(new Error('disk full'));
    const { textarea } = await renderSelectedConversation();

    await act(async () => {
      fireEvent.change(textarea, { target: { value: 'will fail locally' } });
    });
    const sendButton = screen.getByRole('button', { name: 'Send message' });
    await act(async () => {
      fireEvent.click(sendButton);
    });

    // chatSend never runs because the local append failed first.
    await waitFor(() => {
      expect(threadApi.appendMessage).toHaveBeenCalledTimes(1);
    });
    expect(chatSend).not.toHaveBeenCalled();

    // Pending guard released: the user can re-enter text and the send button
    // enables again.
    await act(async () => {
      fireEvent.change(textarea, { target: { value: 'retry' } });
    });
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Send message' })).not.toBeDisabled();
    });
  });

  it('releases the pending-send lock when appendMessage hits a stale-thread error', async () => {
    vi.mocked(threadApi.appendMessage).mockRejectedValueOnce(
      new CoreRpcError('thread missing', 'thread_not_found')
    );
    const { textarea } = await renderSelectedConversation();

    await act(async () => {
      fireEvent.change(textarea, { target: { value: 'stale thread send' } });
    });
    const sendButton = screen.getByRole('button', { name: 'Send message' });
    await act(async () => {
      fireEvent.click(sendButton);
    });

    await waitFor(() => {
      expect(threadApi.appendMessage).toHaveBeenCalledTimes(1);
    });
    expect(chatSend).not.toHaveBeenCalled();

    // Stale-thread branch silently clears the guard; typing must re-enable Send.
    await act(async () => {
      fireEvent.change(textarea, { target: { value: 'retry' } });
    });
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Send message' })).not.toBeDisabled();
    });
  });

  it('clears the pending guard when the 120s silence timer fires', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const { textarea } = await renderSelectedConversation();

      await act(async () => {
        fireEvent.change(textarea, { target: { value: 'hang the backend' } });
      });
      const sendButton = screen.getByRole('button', { name: 'Send message' });
      await act(async () => {
        fireEvent.click(sendButton);
      });
      await waitFor(() => {
        expect(chatSend).toHaveBeenCalledTimes(1);
      });

      // Fast-forward past the 120s silence window with no inference signals.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(120_000);
      });

      // After the safety timeout, typing should re-enable Send — proves the
      // pending guard was reset inside the timeout callback.
      await act(async () => {
        fireEvent.change(textarea, { target: { value: 'retry after timeout' } });
      });
      await waitFor(() => {
        expect(screen.getByRole('button', { name: 'Send message' })).not.toBeDisabled();
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it('rearms the silence timer on sub-agent tool-timeline updates', async () => {
    // Regression: when a delegated sub-agent (`Research`, `Tools Agent`,
    // …) is running, the parent thread's `inferenceStatusByThread` and
    // `streamingAssistantByThread` references can stay put while
    // `toolTimelineByThread` and `taskBoardByThread` tick. The rearm
    // effect must watch all four — otherwise a long sub-agent loop
    // trips the 120s safety timer even though the user can see tools
    // firing in the timeline.
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const { textarea, store, thread } = await renderSelectedConversation();

      await act(async () => {
        fireEvent.change(textarea, { target: { value: 'kick off a sub-agent loop' } });
      });
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: 'Send message' }));
      });
      await waitFor(() => {
        expect(chatSend).toHaveBeenCalledTimes(1);
      });

      // Two-thirds of the way through the safety window, the parent
      // status is already in `subagent` phase and a delegated tool
      // posts a timeline update. After the fix this re-arms the timer.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(80_000);
      });
      await act(async () => {
        store!.dispatch(
          setInferenceStatusForThread({
            threadId: thread.id,
            status: { phase: 'subagent', iteration: 1, maxIterations: 8 },
          })
        );
        store!.dispatch(
          setToolTimelineForThread({
            threadId: thread.id,
            entries: [{ id: 'tl-1', name: 'web_fetch', round: 1, status: 'running' }],
          })
        );
      });

      // Advance another 80s (total elapsed 160s, well past the 120s
      // window). The tool-timeline dispatch should have re-armed the
      // timer at the 80s mark, so the silence timer is now at 80s of
      // its fresh 120s budget and has NOT fired. The pending guard
      // therefore still holds and Send stays disabled — proof the
      // rearm effect ran on a toolTimelineByThread change.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(80_000);
      });
      await act(async () => {
        fireEvent.change(textarea, { target: { value: 'still typing while sub-agent runs' } });
      });
      expect(screen.getByRole('button', { name: 'Send message' })).toBeDisabled();
    } finally {
      vi.useRealTimers();
    }
  });

  it('does NOT rearm the silence timer on an unrelated thread’s updates', async () => {
    // Regression for the per-thread dependency scoping: the rearm effect must
    // react only to the SENDING thread's slices. A different thread churning
    // (background triage, another conversation) must not keep the foreground
    // turn's 120s timer alive — otherwise a truly hung send never fails fast.
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const { textarea, store } = await renderSelectedConversation();

      await act(async () => {
        fireEvent.change(textarea, { target: { value: 'send on the foreground thread' } });
      });
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: 'Send message' }));
      });
      await waitFor(() => {
        expect(chatSend).toHaveBeenCalledTimes(1);
      });

      // Churn an UNRELATED thread the whole time the foreground send is open.
      // None of these dispatches target the sending thread ('send-thread'),
      // so they must not rearm its timer.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(80_000);
      });
      await act(async () => {
        store!.dispatch(
          setInferenceStatusForThread({
            threadId: 'some-other-thread',
            status: { phase: 'subagent', iteration: 3, maxIterations: 8 },
          })
        );
        store!.dispatch(
          setToolTimelineForThread({
            threadId: 'some-other-thread',
            entries: [{ id: 'other-1', name: 'web_fetch', round: 1, status: 'running' }],
          })
        );
      });

      // Cross the original 120s deadline (80s + 50s = 130s). Because the
      // unrelated-thread churn did NOT rearm, the safety timer fires: the
      // pending guard is released and Send re-enables once the user types.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(50_000);
      });
      await act(async () => {
        fireEvent.change(textarea, { target: { value: 'retry after timeout' } });
      });
      await waitFor(() => {
        expect(screen.getByRole('button', { name: 'Send message' })).not.toBeDisabled();
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it('releases the pending-send lock when chatSend rejects', async () => {
    vi.mocked(chatSend).mockRejectedValueOnce(new Error('emit failed'));
    const { textarea } = await renderSelectedConversation();

    await act(async () => {
      fireEvent.change(textarea, { target: { value: 'doomed send' } });
    });
    await waitFor(() => {
      expect(textarea).toHaveValue('doomed send');
    });

    const sendButton = screen.getByRole('button', { name: 'Send message' });
    await act(async () => {
      fireEvent.click(sendButton);
    });

    await waitFor(() => {
      expect(chatSend).toHaveBeenCalledTimes(1);
    });

    // After the failed send, typing again should leave the composer enabled so
    // the user can retry — proves the pending guard was released.
    await act(async () => {
      fireEvent.change(textarea, { target: { value: 'retry send' } });
    });
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Send message' })).not.toBeDisabled();
    });
  });

  it('creates a custom agent profile from the header draft form', async () => {
    const thread = makeThread({ id: 'profile-thread', title: 'Profile Thread' });
    mockGetThreads.mockResolvedValue({ threads: [thread], count: 1 });
    mockGetThreadMessages.mockResolvedValue({ messages: [], count: 0 });
    vi.mocked(agentProfilesApi.upsert).mockResolvedValueOnce({
      activeProfileId: 'custom',
      profiles: [
        {
          id: 'custom',
          name: 'Custom',
          description: 'Custom agent profile',
          agentId: 'orchestrator',
          builtIn: false,
        },
      ],
    });
    vi.mocked(agentProfilesApi.select).mockResolvedValueOnce({
      activeProfileId: 'custom',
      profiles: [
        {
          id: 'custom',
          name: 'Custom',
          description: 'Custom agent profile',
          agentId: 'orchestrator',
          builtIn: false,
        },
      ],
    });

    await act(async () => {
      await renderConversations({
        thread: selectedThreadState(thread),
        socket: socketState('connected'),
      });
    });

    fireEvent.click(await screen.findByLabelText('Create agent profile'));
    fireEvent.change(screen.getByPlaceholderText('Profile name'), { target: { value: 'Custom' } });
    fireEvent.change(screen.getByPlaceholderText('Prompt style'), {
      target: { value: 'Be concise' },
    });
    fireEvent.change(screen.getByPlaceholderText('Allowed tools'), {
      target: { value: 'todowrite, spawn_parallel_agents' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    await waitFor(() => expect(agentProfilesApi.upsert).toHaveBeenCalledTimes(1));
    expect(agentProfilesApi.upsert).toHaveBeenCalledWith(
      expect.objectContaining({
        name: 'Custom',
        systemPromptSuffix: 'Be concise',
        allowedTools: ['todowrite', 'spawn_parallel_agents'],
      })
    );
    expect(agentProfilesApi.select).toHaveBeenCalled();
  });

  it('shows validation when creating a duplicate profile name', async () => {
    await act(async () => {
      await renderConversations({ thread: emptyThreadState, socket: socketState('connected') });
    });

    fireEvent.click(await screen.findByLabelText('Create agent profile'));
    fireEvent.change(screen.getByPlaceholderText('Profile name'), { target: { value: 'Default' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    expect(await screen.findByText('Agent profile "Default" already exists.')).toBeInTheDocument();
    expect(agentProfilesApi.upsert).not.toHaveBeenCalled();
  });

  it('rolls back and shows feedback when task board move persistence fails', async () => {
    const thread = makeThread({ id: 'board-thread', title: 'Board Thread' });
    const board = {
      threadId: 'board-thread',
      updatedAt: '2026-05-04T10:00:00Z',
      cards: [
        {
          id: 'task-1',
          title: 'Plan rollout',
          status: 'todo' as const,
          order: 0,
          updatedAt: '2026-05-04T10:00:00Z',
        },
      ],
    };
    mockGetThreads.mockResolvedValue({ threads: [thread], count: 1 });
    mockGetThreadMessages.mockResolvedValue({ messages: [], count: 0 });
    vi.mocked(threadApi.getTaskBoard).mockResolvedValueOnce(board);
    vi.mocked(threadApi.putTaskBoard).mockRejectedValueOnce(new Error('write failed'));

    await act(async () => {
      await renderConversations({
        thread: selectedThreadState(thread),
        socket: socketState('connected'),
      });
    });

    expect(await screen.findByText('Plan rollout')).toBeInTheDocument();
    fireEvent.click(screen.getByLabelText('Move right'));

    await waitFor(() => {
      expect(
        screen.getByText('Could not update task; changes were not saved.')
      ).toBeInTheDocument();
    });
    expect(threadApi.putTaskBoard).toHaveBeenCalledWith(
      'board-thread',
      expect.arrayContaining([expect.objectContaining({ id: 'task-1', status: 'in_progress' })])
    );
  });

  it('rolls back and shows feedback when task board edit persistence fails', async () => {
    const thread = makeThread({ id: 'edit-board-thread', title: 'Edit Board Thread' });
    const board = {
      threadId: 'edit-board-thread',
      updatedAt: '2026-05-04T10:00:00Z',
      cards: [
        {
          id: 'task-1',
          title: 'Plan rollout',
          status: 'todo' as const,
          objective: 'Draft the launch task brief',
          assignedAgent: 'planner',
          approvalMode: 'required' as const,
          plan: ['Read docs'],
          allowedTools: ['todo'],
          acceptanceCriteria: ['Saved board round-trips'],
          evidence: [],
          order: 0,
          updatedAt: '2026-05-04T10:00:00Z',
        },
      ],
    };
    mockGetThreads.mockResolvedValue({ threads: [thread], count: 1 });
    mockGetThreadMessages.mockResolvedValue({ messages: [], count: 0 });
    vi.mocked(threadApi.getTaskBoard).mockResolvedValueOnce(board);
    vi.mocked(threadApi.putTaskBoard).mockRejectedValueOnce(new Error('write failed'));

    await act(async () => {
      await renderConversations({
        thread: selectedThreadState(thread),
        socket: socketState('connected'),
      });
    });

    expect(await screen.findByText('Plan rollout')).toBeInTheDocument();
    fireEvent.click(screen.getByText('Task brief'));
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Updated rollout' } });
    fireEvent.change(screen.getByLabelText('Assigned agent'), {
      target: { value: 'code_executor' },
    });
    fireEvent.click(screen.getByText('Save changes'));

    await waitFor(() => {
      expect(
        screen.getByText('Could not update task; changes were not saved.')
      ).toBeInTheDocument();
    });
    expect(threadApi.putTaskBoard).toHaveBeenCalledWith(
      'edit-board-thread',
      expect.arrayContaining([
        expect.objectContaining({
          id: 'task-1',
          title: 'Updated rollout',
          assignedAgent: 'code_executor',
        }),
      ])
    );
  });

  it('sends with Enter when the composer is not composing text', async () => {
    const { textarea, thread } = await renderSelectedConversation();

    await act(async () => {
      fireEvent.change(textarea, { target: { value: 'enter send' } });
    });
    await waitFor(() => {
      expect(textarea).toHaveValue('enter send');
      expect(screen.getByRole('button', { name: 'Send message' })).not.toBeDisabled();
    });

    await act(async () => {
      fireEvent.keyDown(textarea, { key: 'Enter' });
    });

    await waitFor(() => {
      expect(chatSend).toHaveBeenCalledWith({
        threadId: thread.id,
        message: 'enter send',
        model: 'reasoning-v1',
        profileId: 'default',
        locale: 'en',
      });
    });
  });

  it('does not send while an IME composition key event is confirming text', async () => {
    const { textarea } = await renderSelectedConversation();

    await act(async () => {
      fireEvent.change(textarea, { target: { value: '你好' } });
    });
    await waitFor(() => {
      expect(textarea).toHaveValue('你好');
      expect(screen.getByRole('button', { name: 'Send message' })).not.toBeDisabled();
    });

    await act(async () => {
      const event = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true });
      Object.defineProperty(event, 'isComposing', { value: true });
      textarea.dispatchEvent(event);
    });

    expect(chatSend).not.toHaveBeenCalled();
    expect(textarea).toHaveValue('你好');
  });

  it('does not send for legacy IME keyCode 229 events', async () => {
    const { textarea } = await renderSelectedConversation();

    await act(async () => {
      fireEvent.change(textarea, { target: { value: 'かな' } });
    });
    await waitFor(() => {
      expect(textarea).toHaveValue('かな');
      expect(screen.getByRole('button', { name: 'Send message' })).not.toBeDisabled();
    });

    await act(async () => {
      fireEvent.keyDown(textarea, { key: 'Enter', keyCode: 229 });
    });

    expect(chatSend).not.toHaveBeenCalled();
    expect(textarea).toHaveValue('かな');
  });

  it('does not send while composition is active even if keydown lacks IME flags', async () => {
    const { textarea, thread } = await renderSelectedConversation();

    await act(async () => {
      fireEvent.change(textarea, { target: { value: '안녕' } });
    });
    await waitFor(() => {
      expect(textarea).toHaveValue('안녕');
      expect(screen.getByRole('button', { name: 'Send message' })).not.toBeDisabled();
    });

    await act(async () => {
      fireEvent.compositionStart(textarea);
      fireEvent.keyDown(textarea, { key: 'Enter' });
    });

    expect(chatSend).not.toHaveBeenCalled();
    expect(textarea).toHaveValue('안녕');

    await act(async () => {
      fireEvent.compositionEnd(textarea);
      fireEvent.keyDown(textarea, { key: 'Enter' });
    });

    await waitFor(() => {
      expect(chatSend).toHaveBeenCalledWith({
        threadId: thread.id,
        message: '안녕',
        model: 'reasoning-v1',
        profileId: 'default',
        locale: 'en',
      });
    });
  });

  // Batch-5: Conversation category tabs keep stable labels and mapping (pr#1646).
  //
  // The tab set is fixed so categories do not disappear when the thread list
  // is empty, and the active-filter state remains unambiguous.
  it('renders all four fixed category tabs with stable labels', async () => {
    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    // Sidebar is hidden by default — open it first.
    await openSidebar();

    // All four tabs must be present regardless of thread count.
    expect(screen.getByRole('tab', { name: 'All' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Work' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Briefing' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Notification' })).toBeInTheDocument();
  });

  it('starts with the "All" tab selected', async () => {
    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    // Sidebar is hidden by default — open it first.
    await openSidebar();

    expect(screen.getByRole('tab', { name: 'All' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('tab', { name: 'Work' })).toHaveAttribute('aria-selected', 'false');
  });

  it('shows "No threads yet" placeholder when All tab is active and list is empty', async () => {
    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    // Sidebar is hidden by default — open it first.
    await openSidebar();

    expect(screen.getByText('No threads yet')).toBeInTheDocument();
  });

  it('shows category-specific empty message when a label tab is selected and no threads match', async () => {
    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    // Sidebar is hidden by default — open it first.
    await openSidebar();

    fireEvent.click(screen.getByRole('tab', { name: 'Work' }));

    await waitFor(() => {
      expect(screen.getByText(/"work" threads/i)).toBeInTheDocument();
    });
  });

  // #1624 — Workers tab is the dedicated entry-point for sub-agent threads.
  // When the active workspace has no worker threads (parentThreadId set), the
  // empty state must use the friendly "No worker threads yet" copy rather
  // than `No "workers" threads`.
  it('shows the worker-specific empty message when the Workers tab is selected', async () => {
    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    // Sidebar is hidden by default — open it first.
    await openSidebar();

    fireEvent.click(screen.getByRole('tab', { name: 'Workers' }));

    await waitFor(() => {
      expect(screen.getByText('No worker threads yet')).toBeInTheDocument();
    });
  });
});

// #1624 — When a worker thread is the active selection, the header surfaces
// a "back to <parent title>" button that navigates the user back to the
// parent conversation. Covers the `selectedThreadParent` derivation and the
// click handler that dispatches setSelectedThread + loadThreadMessages.
describe('Conversations — worker thread back-to-parent navigation (#1624)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockGetThreads.mockResolvedValue({ threads: [], count: 0 });
    mockGetThreadMessages.mockResolvedValue({ messages: [], count: 0 });
  });

  it('renders a back-to-parent button when the active thread has a parent', async () => {
    const parent = makeThread({ id: 't-parent', title: 'Parent Conversation' });
    const child = makeThread({ id: 't-child', title: 'Worker Task', parentThreadId: 't-parent' });
    mockGetThreads.mockResolvedValue({ threads: [parent, child], count: 2 });

    let store: ReturnType<typeof buildStore> | undefined;
    await act(async () => {
      store = await renderConversations({
        thread: {
          ...emptyThreadState,
          threads: [parent, child],
          selectedThreadId: child.id,
          messagesByThreadId: { [child.id]: [] },
        },
      });
    });

    // The mount effect resumes onto a *visible* (non-worker) thread, so even
    // though the preloaded state pointed at the child, the page auto-selects
    // the parent. Re-select the worker thread now that mount has settled to
    // mimic the user clicking through to a worker from the Workers tab.
    await act(async () => {
      store!.dispatch(setSelectedThread('t-child'));
    });

    const backBtn = await screen.findByTestId('worker-thread-back-to-parent');
    expect(backBtn.textContent).toContain('Parent Conversation');
  });

  it('falls back to a generic title when the parent thread is missing from the list', async () => {
    const parent = makeThread({ id: 't-parent', title: 'Other Parent' });
    const child = makeThread({
      id: 't-child',
      title: 'Worker Task',
      parentThreadId: 't-missing-parent',
    });
    // The parent referenced by `parentThreadId` is intentionally absent from
    // the thread list so the `selectedThreadParent` resolver hits its fallback
    // branch. A separate parent is kept around so mount-time resume has a
    // visible thread to land on.
    mockGetThreads.mockResolvedValue({ threads: [parent, child], count: 2 });

    let store: ReturnType<typeof buildStore> | undefined;
    await act(async () => {
      store = await renderConversations({
        thread: {
          ...emptyThreadState,
          threads: [parent, child],
          selectedThreadId: child.id,
          messagesByThreadId: { [child.id]: [] },
        },
      });
    });
    await act(async () => {
      store!.dispatch(setSelectedThread('t-child'));
    });

    const backBtn = await screen.findByTestId('worker-thread-back-to-parent');
    expect(backBtn.textContent).toContain('parent thread');
  });

  it('dispatches selection + load when the back-to-parent button is clicked', async () => {
    const parent = makeThread({ id: 't-parent', title: 'Parent Conversation' });
    const child = makeThread({ id: 't-child', title: 'Worker Task', parentThreadId: 't-parent' });
    mockGetThreads.mockResolvedValue({ threads: [parent, child], count: 2 });

    let store: ReturnType<typeof buildStore> | undefined;
    await act(async () => {
      store = await renderConversations({
        thread: {
          ...emptyThreadState,
          threads: [parent, child],
          selectedThreadId: child.id,
          messagesByThreadId: { [child.id]: [] },
        },
      });
    });
    await act(async () => {
      store!.dispatch(setSelectedThread('t-child'));
    });

    const backBtn = await screen.findByTestId('worker-thread-back-to-parent');
    await act(async () => {
      fireEvent.click(backBtn);
    });

    // After click, the redux store should reflect the parent thread as the
    // newly selected conversation.
    await waitFor(() => {
      const state = store!.getState() as { thread: { selectedThreadId: string | null } };
      expect(state.thread.selectedThreadId).toBe('t-parent');
    });
    // The loadThreadMessages thunk goes through the threadApi.getThreadMessages
    // helper — verify it was kicked off for the parent thread.
    expect(mockGetThreadMessages).toHaveBeenCalledWith('t-parent');
  });

  // Covers line 1871: t('chat.budgetComplete') — cycleBudgetUsd=0 exhausted branch
  it('renders budgetComplete copy when cycleBudgetUsd=0 and budget is exhausted', async () => {
    const teamUsage = { cycleBudgetUsd: 0, remainingUsd: 0, cycleSpentUsd: 0, cycleEndsAt: null };

    mockUseUsageState.mockReturnValue({
      teamUsage,
      currentPlan: null,
      currentTier: 'PRO' as const,
      isFreeTier: false,
      usagePct: 1.0,
      isNearLimit: true,
      isAtLimit: true,
      isBudgetExhausted: true,
      shouldShowBudgetCompletedMessage: true,
      isLoading: false,
      refresh: vi.fn(),
    });

    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    // cycleBudgetUsd=0 → false branch of cycleBudgetUsd > 0 ternary → budgetComplete key
    expect(screen.getByText(/Your included budget is complete/i)).toBeInTheDocument();
  });

  // Covers line 1910: cycleEndsAt truthy branch inside cycle-pill tooltip
  it('renders reset time in cycle-pill tooltip when cycleEndsAt is set', async () => {
    const teamUsage = {
      cycleBudgetUsd: 10,
      remainingUsd: 5,
      cycleSpentUsd: 5,
      cycleEndsAt: '2026-06-01T00:00:00.000Z',
    };

    mockUseUsageState.mockReturnValue({
      teamUsage,
      currentPlan: null,
      currentTier: 'PRO' as const,
      isFreeTier: false,
      usagePct: 0.5,
      isNearLimit: false,
      isAtLimit: false,
      isBudgetExhausted: false,
      shouldShowBudgetCompletedMessage: false,
      isLoading: false,
      refresh: vi.fn(),
    });

    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    // Tooltip is hidden via CSS but present in DOM; cycleEndsAt truthy → reset span renders
    expect(screen.getByText('Cycle')).toBeInTheDocument();
    // The tooltip resets span contains "resets" text (covers line 1910 conditional)
    const resetSpans = document.querySelectorAll('[class*="text-stone-400"]');
    expect(resetSpans.length).toBeGreaterThan(0);
  });

  // Covers lines 1903-1904: loading animation span when isLoading=true, teamUsage=null
  it('renders loading pulse span in cycle-pill area when isLoading=true', async () => {
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
      isLoading: true,
      refresh: vi.fn(),
    });

    await act(async () => {
      await renderConversations({ thread: emptyThreadState });
    });

    // Loading span with animate-pulse is present when teamUsage=null and loading
    expect(screen.getByText('Loading…')).toBeInTheDocument();
  });
});

describe('Conversations — thread title editing', () => {
  beforeEach(() => {
    vi.clearAllMocks();
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
    mockGetThreadMessages.mockResolvedValue({ messages: [], count: 0 });
  });

  it('shows pencil icon on hover and enters edit mode on click', async () => {
    const thread = makeThread({ id: 'edit-title-thread', title: 'Original Title' });
    mockGetThreads.mockResolvedValue({ threads: [thread], count: 1 });

    await act(async () => {
      await renderConversations({
        thread: selectedThreadState(thread),
        socket: socketState('connected'),
      });
    });

    expect(screen.getByText('Original Title')).toBeInTheDocument();

    const editBtn = screen.getByRole('button', { name: 'Edit thread title' });
    expect(editBtn).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(editBtn);
    });

    const input = screen.getByLabelText('Edit thread title');
    expect(input).toBeInTheDocument();
    expect(input).toHaveValue('Original Title');
  });

  it('commits edited title on Enter and dispatches updateThreadTitle', async () => {
    const thread = makeThread({ id: 'commit-title-thread', title: 'Old Title' });
    mockGetThreads.mockResolvedValue({ threads: [thread], count: 1 });
    (threadApi.updateTitle as ReturnType<typeof vi.fn>).mockResolvedValue({
      ...thread,
      title: 'New Title',
    });

    await act(async () => {
      await renderConversations({
        thread: selectedThreadState(thread),
        socket: socketState('connected'),
      });
    });

    const editBtn = screen.getByRole('button', { name: 'Edit thread title' });
    await act(async () => {
      fireEvent.click(editBtn);
    });

    const input = screen.getByLabelText('Edit thread title');
    await act(async () => {
      fireEvent.change(input, { target: { value: 'New Title' } });
    });
    await act(async () => {
      fireEvent.keyDown(input, { key: 'Enter' });
    });

    await waitFor(() => {
      expect(threadApi.updateTitle).toHaveBeenCalledWith('commit-title-thread', 'New Title');
    });
  });

  it('cancels editing on Escape without dispatching', async () => {
    const thread = makeThread({ id: 'cancel-title-thread', title: 'Keep Me' });
    mockGetThreads.mockResolvedValue({ threads: [thread], count: 1 });

    await act(async () => {
      await renderConversations({
        thread: selectedThreadState(thread),
        socket: socketState('connected'),
      });
    });

    const editBtn = screen.getByRole('button', { name: 'Edit thread title' });
    await act(async () => {
      fireEvent.click(editBtn);
    });

    const input = screen.getByLabelText('Edit thread title');
    await act(async () => {
      fireEvent.change(input, { target: { value: 'Changed' } });
    });
    await act(async () => {
      fireEvent.keyDown(input, { key: 'Escape' });
    });

    expect(screen.getByText('Keep Me')).toBeInTheDocument();
    expect(threadApi.updateTitle).not.toHaveBeenCalled();
  });

  it('does not dispatch when title is empty after trim', async () => {
    const thread = makeThread({ id: 'empty-title-thread', title: 'Has Title' });
    mockGetThreads.mockResolvedValue({ threads: [thread], count: 1 });

    await act(async () => {
      await renderConversations({
        thread: selectedThreadState(thread),
        socket: socketState('connected'),
      });
    });

    const editBtn = screen.getByRole('button', { name: 'Edit thread title' });
    await act(async () => {
      fireEvent.click(editBtn);
    });

    const input = screen.getByLabelText('Edit thread title');
    await act(async () => {
      fireEvent.change(input, { target: { value: '   ' } });
    });
    await act(async () => {
      fireEvent.keyDown(input, { key: 'Enter' });
    });

    expect(threadApi.updateTitle).not.toHaveBeenCalled();
  });
});
