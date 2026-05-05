/**
 * Tests for OnboardingLayout — verifies that completeAndExit:
 *  - does NOT create a welcome thread (welcome-agent replaced by Joyride walkthrough)
 *  - does NOT call chatSend
 *  - DOES set the walkthrough pending flag in localStorage
 *  - DOES call setOnboardingCompletedFlag(true)
 *
 * [#1123] Old assertions about welcome thread creation were replaced.
 */
import { configureStore } from '@reduxjs/toolkit';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import socketReducer from '../../../store/socketSlice';
import threadReducer from '../../../store/threadSlice';
import { useOnboardingContext } from '../OnboardingContext';

// ── Module-level mocks ─────────────────────────────────────────────────────

// [#1123] Mock setWalkthroughPending to allow per-test override (e.g. throw),
// while writing to localStorage by default so existing assertions still pass.
// Covers the catch block in completeAndExit (OnboardingLayout.tsx:138).
const mockSetWalkthroughPending = vi.fn(() => {
  localStorage.setItem('openhuman:walkthrough_pending', 'true');
});
vi.mock('../../../components/walkthrough/AppWalkthrough', () => ({
  setWalkthroughPending: () => mockSetWalkthroughPending(),
}));

vi.mock('../../../providers/CoreStateProvider', () => ({ useCoreState: vi.fn() }));

vi.mock('../../../services/api/userApi', () => ({
  userApi: { onboardingComplete: vi.fn().mockResolvedValue(undefined) },
}));

// [#1123] chatSend should NOT be called — walkthrough replaced welcome-agent
vi.mock('../../../services/chatService', () => ({
  chatSend: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('../../../utils/toolDefinitions', () => ({ getDefaultEnabledTools: vi.fn(() => []) }));

vi.mock('../components/BetaBanner', () => ({ default: () => <div data-testid="beta-banner" /> }));

// ── Spy on threadApi ───────────────────────────────────────────────────────

const mockCreateNewThreadArg = vi.fn();

vi.mock('../../../services/api/threadApi', () => ({
  threadApi: {
    createNewThread: (labels: string[]) => {
      mockCreateNewThreadArg(labels);
      return Promise.resolve({ id: 'welcome-thread-id', labels });
    },
    getThreads: vi.fn().mockResolvedValue({ threads: [], count: 0 }),
    getThreadMessages: vi.fn().mockResolvedValue({ messages: [], count: 0 }),
    appendMessage: vi.fn().mockResolvedValue({}),
    deleteThread: vi.fn().mockResolvedValue({ deleted: true }),
    generateTitleIfNeeded: vi.fn().mockResolvedValue({}),
    updateMessage: vi.fn().mockResolvedValue({}),
    purge: vi.fn().mockResolvedValue({}),
    updateLabels: vi.fn().mockResolvedValue({}),
  },
}));

// ── A minimal child component that calls completeAndExit ───────────────────

function TriggerComplete() {
  const { completeAndExit } = useOnboardingContext();
  return (
    <button onClick={() => void completeAndExit()} data-testid="complete-btn">
      Complete
    </button>
  );
}

// ── Helpers ────────────────────────────────────────────────────────────────

function buildStore() {
  return configureStore({
    reducer: { thread: threadReducer, socket: socketReducer },
    preloadedState: {
      thread: {
        threads: [],
        selectedThreadId: null,
        welcomeThreadId: null,
        activeThreadId: null,
        messagesByThreadId: {},
        messages: [],
        isLoadingThreads: false,
        isLoadingMessages: false,
        messagesError: null,
      },
    } as unknown as Parameters<typeof configureStore>[0]['preloadedState'],
  });
}

async function setupLayout() {
  const { useCoreState } = await import('../../../providers/CoreStateProvider');

  const mockSetOnboardingCompletedFlag = vi.fn().mockResolvedValue(undefined);
  const mockSetOnboardingTasks = vi.fn().mockResolvedValue(undefined);

  vi.mocked(useCoreState).mockReturnValue({
    snapshot: {
      auth: { isAuthenticated: true, userId: 'u1', user: null, profileId: null },
      sessionToken: null,
      currentUser: null,
      onboardingCompleted: false,
      chatOnboardingCompleted: false,
      analyticsEnabled: false,
      localState: { encryptionKey: null, primaryWalletAddress: null, onboardingTasks: null },
      runtime: { screenIntelligence: null, localAi: null, autocomplete: null, service: null },
    },
    isBootstrapping: false,
    isReady: true,
    teams: [],
    teamMembersById: {},
    teamInvitesById: {},
    setOnboardingCompletedFlag: mockSetOnboardingCompletedFlag,
    setOnboardingTasks: mockSetOnboardingTasks,
    refreshSnapshot: vi.fn(),
  } as never);

  const { default: OnboardingLayout } = await import('../OnboardingLayout');
  const store = buildStore();

  render(
    <Provider store={store}>
      <MemoryRouter initialEntries={['/onboarding']}>
        <Routes>
          <Route path="/onboarding" element={<OnboardingLayout />}>
            <Route index element={<TriggerComplete />} />
          </Route>
          <Route path="/home" element={<div data-testid="home-page" />} />
        </Routes>
      </MemoryRouter>
    </Provider>
  );

  return { store, mockSetOnboardingCompletedFlag, mockSetOnboardingTasks };
}

// ── Tests ──────────────────────────────────────────────────────────────────

describe('OnboardingLayout — Joyride walkthrough integration (#1123)', () => {
  beforeEach(() => {
    mockCreateNewThreadArg.mockClear();
    // Reset call history only — restore the default implementation (writes localStorage)
    mockSetWalkthroughPending.mockClear();
    localStorage.clear();
  });

  // [#1123] Replaced old test: no welcome thread creation
  it('does NOT create a welcome thread on completeAndExit', async () => {
    await setupLayout();

    await act(async () => {
      fireEvent.click(screen.getByTestId('complete-btn'));
    });

    // [#1123] Welcome thread creation is no longer part of the flow
    expect(mockCreateNewThreadArg).not.toHaveBeenCalled();
  });

  it('calls setOnboardingCompletedFlag(true) during completeAndExit', async () => {
    const { mockSetOnboardingCompletedFlag } = await setupLayout();

    await act(async () => {
      fireEvent.click(screen.getByTestId('complete-btn'));
    });

    expect(mockSetOnboardingCompletedFlag).toHaveBeenCalledWith(true);
  });

  it('sets the walkthrough pending flag in localStorage after completeAndExit', async () => {
    await setupLayout();

    await act(async () => {
      fireEvent.click(screen.getByTestId('complete-btn'));
    });

    // [#1123] Walkthrough pending flag should be set instead of welcome thread
    expect(localStorage.getItem('openhuman:walkthrough_pending')).toBe('true');
  });

  // [#1123] Old test — welcome thread in Redux state — replaced:
  // it('records the welcome thread id in the Redux store after thread creation', ...)
  // The welcome thread is no longer stored in Redux.
  it('does NOT set welcomeThreadId in Redux store on completeAndExit', async () => {
    const { store } = await setupLayout();

    await act(async () => {
      fireEvent.click(screen.getByTestId('complete-btn'));
    });

    const { thread } = store.getState() as { thread: { welcomeThreadId: string | null } };
    expect(thread.welcomeThreadId).toBeNull();
  });

  // [#1123] Explicit guard: chatSend must never be called in the Joyride flow
  it('does NOT call chatSend on completeAndExit', async () => {
    await setupLayout();
    const { chatSend } = await import('../../../services/chatService');

    await act(async () => {
      fireEvent.click(screen.getByTestId('complete-btn'));
    });

    expect(chatSend).not.toHaveBeenCalled();
  });

  // Covers the catch branch in completeAndExit (OnboardingLayout.tsx:138):
  // when setWalkthroughPending throws, navigation still proceeds to /home.
  it('still navigates to /home when setWalkthroughPending throws', async () => {
    // Override default impl to throw for this one test invocation
    mockSetWalkthroughPending.mockImplementationOnce(() => {
      throw new Error('storage unavailable');
    });
    await setupLayout();

    await act(async () => {
      fireEvent.click(screen.getByTestId('complete-btn'));
    });

    // Navigation should still proceed even when the flag cannot be written.
    expect(screen.getByTestId('home-page')).toBeInTheDocument();
  });
});
