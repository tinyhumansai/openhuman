/**
 * Tests for OnboardingLayout — specifically verifies that line 97 (the
 * createNewThread call with the ONBOARDING_WELCOME_THREAD_LABEL) is executed
 * when `completeAndExit` runs successfully.
 */
import { configureStore } from '@reduxjs/toolkit';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { ONBOARDING_WELCOME_THREAD_LABEL } from '../../../constants/onboardingChat';
import socketReducer from '../../../store/socketSlice';
import threadReducer from '../../../store/threadSlice';
import { useOnboardingContext } from '../OnboardingContext';

// ── Module-level mocks ─────────────────────────────────────────────────────

vi.mock('../../../providers/CoreStateProvider', () => ({ useCoreState: vi.fn() }));

vi.mock('../../../services/api/userApi', () => ({
  userApi: { onboardingComplete: vi.fn().mockResolvedValue(undefined) },
}));

vi.mock('../../../services/chatService', () => ({
  chatSend: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('../../../utils/toolDefinitions', () => ({ getDefaultEnabledTools: vi.fn(() => []) }));

vi.mock('../components/BetaBanner', () => ({ default: () => <div data-testid="beta-banner" /> }));

// ── Spy on threadSlice actions dispatched ──────────────────────────────────

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

describe('OnboardingLayout — createNewThread with onboarding label', () => {
  beforeEach(() => {
    mockCreateNewThreadArg.mockClear();
  });

  it('calls createNewThread with the onboarding welcome label on completeAndExit', async () => {
    await setupLayout();

    await act(async () => {
      fireEvent.click(screen.getByTestId('complete-btn'));
    });

    expect(mockCreateNewThreadArg).toHaveBeenCalledWith([ONBOARDING_WELCOME_THREAD_LABEL]);
  });

  it('calls setOnboardingCompletedFlag(true) during completeAndExit', async () => {
    const { mockSetOnboardingCompletedFlag } = await setupLayout();

    await act(async () => {
      fireEvent.click(screen.getByTestId('complete-btn'));
    });

    expect(mockSetOnboardingCompletedFlag).toHaveBeenCalledWith(true);
  });

  it('records the welcome thread id in the Redux store after thread creation', async () => {
    const { store } = await setupLayout();

    await act(async () => {
      fireEvent.click(screen.getByTestId('complete-btn'));
    });

    // The dispatch(setWelcomeThreadId(newThread.id)) should have updated state
    const { thread } = store.getState() as { thread: { welcomeThreadId: string | null } };
    expect(thread.welcomeThreadId).toBe('welcome-thread-id');
  });
});
