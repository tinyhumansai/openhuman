/**
 * The agent-process-source command is offered on every chat surface.
 *
 * `showProcessSource` only drives `TranscriptOverlays`, which mounts inside the
 * assistant-ui panel. That panel used to be one half of an either/or — voice
 * (`mic-cloud`) mode mounted a separate legacy transcript instead, where the
 * state the command set had no host, so the command had to be disabled there.
 * Voice mode now renders the same assistant-ui panel with only the composer
 * swapped, so the overlays (and the command) are live in both modes.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { SidebarSlotOutlet, SidebarSlotProvider } from '../../components/layout/shell/SidebarSlot';
import { registry } from '../../lib/commands/registry';
import chatRuntimeReducer from '../../store/chatRuntimeSlice';
import layoutReducer from '../../store/layoutSlice';
import runModeReducer from '../../store/runModeSlice';
import socketReducer from '../../store/socketSlice';
import themeReducer from '../../store/themeSlice';
import threadGoalReducer from '../../store/threadGoalSlice';
import threadReducer from '../../store/threadSlice';
import threadTodosReducer from '../../store/threadTodosSlice';
import type { Thread } from '../../types/thread';
import Conversations from './Conversations';

const { mockGetThreads, mockGetThreadMessages, mockUseUsageState, mockChatSend } = vi.hoisted(
  () => ({
    mockGetThreads: vi.fn().mockResolvedValue({ threads: [], count: 0 }),
    mockGetThreadMessages: vi.fn().mockResolvedValue({ messages: [], count: 0 }),
    mockChatSend: vi.fn().mockResolvedValue(undefined),
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
  chatCancel: vi.fn().mockResolvedValue({ accepted: true, turnCancelled: true }),
  chatClearQueue: vi.fn().mockResolvedValue(0),
  chatSend: mockChatSend,
  subscribeChatEvents: vi.fn(() => () => {}),
  useRustChat: vi.fn(() => true),
}));

vi.mock('../../components/chat/ModelQualityPill', async importOriginal => {
  const actual = await importOriginal<typeof import('../../components/chat/ModelQualityPill')>();
  return { ...actual, useModelPickerProviders: () => ({ providers: [], loading: false }) };
});

vi.mock('../../components/settings/panels/ai/ProviderModelPickerDialog', () => ({
  ProviderModelPickerDialog: ({
    onSelect,
  }: {
    onSelect: (selection: {
      source: { kind: 'cloud'; providerSlug: string };
      model: string;
    }) => void;
  }) => (
    <div data-testid="provider-model-picker-dialog">
      <button
        type="button"
        onClick={() =>
          onSelect({ source: { kind: 'cloud', providerSlug: 'huggingface' }, model: 'org/model' })
        }>
        Pick Hugging Face model
      </button>
    </div>
  ),
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
      threadTodos: threadTodosReducer,
      threadGoal: threadGoalReducer,
      runMode: runModeReducer,
    }),
    preloadedState: preload as never,
  });
}

async function renderChat(
  composer?: 'text' | 'mic-cloud',
  withProcessData = false,
  active = false
) {
  mockGetThreads.mockResolvedValue({ threads: [thread], count: 1 });
  const store = buildStore({
    thread: {
      threads: [thread],
      selectedThreadId: THREAD_ID,
      activeThreadIds: active ? { [THREAD_ID]: true } : {},
      welcomeThreadId: null,
      messagesByThreadId: { [THREAD_ID]: [] },
      messages: [],
      isLoadingThreads: false,
      isLoadingMessages: false,
      messagesError: null,
    },
    socket: { byUser: { __pending__: { status: 'connected', socketId: 'socket-1' } } },
    ...(withProcessData
      ? {
          chatRuntime: {
            ...chatRuntimeReducer(undefined, { type: '@@init' }),
            toolTimelineByThread: {
              [THREAD_ID]: [{ id: 'c1', name: 'web_fetch', round: 1, seq: 0, status: 'success' }],
            },
          },
        }
      : {}),
  });
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

async function submitComposerText(text: string) {
  const input = screen.getByRole('textbox');
  await act(async () => {
    input.textContent = text;
    fireEvent.input(input, { data: text, inputType: 'insertText' });
  });
  const sendButton = screen.getByTestId('send-message-button');
  await waitFor(() => expect(sendButton).not.toBeDisabled());
  await act(async () => {
    fireEvent.click(sendButton);
  });
  await waitFor(() => expect(mockChatSend).toHaveBeenCalledTimes(1));
}

async function selectPickerModel() {
  fireEvent.click(screen.getByTestId('composer-chat-settings'));
  fireEvent.click(await screen.findByRole('button', { name: 'Pick Hugging Face model' }));
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

  it('is disabled when the assistant-ui surface has no process data to show', async () => {
    await renderChat('text');

    const action = registry.getAction(ACTION_ID);
    expect(action, 'the command must be registered on the text composer').toBeDefined();
    expect(action?.enabled?.()).toBe(false);
    // The palette runs it through `runAction`, which re-checks `enabled`.
    expect(registry.runAction(ACTION_ID)).toBe(false);
  });

  it('is enabled in mic-cloud voice mode too, which renders the same assistant-ui panel', async () => {
    // With something to show: the command is gated on process data (above),
    // and voice mode must not add a gate of its own.
    await renderChat('mic-cloud', true);

    // Voice mode swaps only the composer: the transcript is the assistant-ui
    // viewport and the text composer is replaced by the voice composer.
    expect(document.querySelector('[data-slot="aui_thread-viewport"]')).not.toBeNull();
    expect(document.querySelector('[data-testid="voice-composer"]')).not.toBeNull();
    expect(document.querySelector('[data-slot="aui_composer-shell"]')).toBeNull();

    const action = registry.getAction(ACTION_ID);
    expect(action, 'the command is registered in voice mode').toBeDefined();
    expect(action?.enabled?.()).toBe(true);
    expect(registry.runAction(ACTION_ID)).toBe(true);
  });
});

describe('composer model routing', () => {
  afterEach(() => {
    cleanup();
    registry.reset();
  });

  it('leaves the persisted model to the core for a normal send by default', async () => {
    mockChatSend.mockClear();
    await renderChat('text');
    await submitComposerText('normal default route');

    expect(mockChatSend.mock.calls[0][0]).not.toHaveProperty('model');
  });

  it('leaves the persisted model to the core for a follow-up send by default', async () => {
    mockChatSend.mockClear();
    await renderChat('text', false, true);
    await submitComposerText('follow-up default route');

    expect(mockChatSend.mock.calls[0][0]).toMatchObject({ queueMode: 'followup' });
    expect(mockChatSend.mock.calls[0][0]).not.toHaveProperty('model');
  });

  it('forwards the concrete provider/model chosen in the picker for a normal send', async () => {
    mockChatSend.mockClear();
    await renderChat('text');

    await selectPickerModel();
    await submitComposerText('explicit picker route');

    expect(mockChatSend.mock.calls[0][0]).toMatchObject({ model: 'huggingface:org/model' });
  });

  it('forwards the concrete provider/model chosen in the picker for a follow-up send', async () => {
    mockChatSend.mockClear();
    await renderChat('text', false, true);

    await selectPickerModel();
    await submitComposerText('explicit follow-up picker route');

    expect(mockChatSend.mock.calls[0][0]).toMatchObject({
      model: 'huggingface:org/model',
      queueMode: 'followup',
    });
  });
});
