/**
 * Attachment feature tests for Conversations.tsx — covers the new lines added
 * for multimodal image attachments: handleAttachFiles, error display,
 * attachment-only sends, and user bubble image rendering.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import agentProfileReducer from '../../store/agentProfileSlice';
import chatRuntimeReducer from '../../store/chatRuntimeSlice';
import socketReducer from '../../store/socketSlice';
import threadReducer from '../../store/threadSlice';
import type { Thread } from '../../types/thread';

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
    select: vi.fn().mockResolvedValue({ activeProfileId: 'default', profiles: [] }),
    upsert: vi.fn().mockResolvedValue({ activeProfileId: 'default', profiles: [] }),
    delete: vi.fn().mockResolvedValue({ activeProfileId: 'default', profiles: [] }),
  },
}));

vi.mock('../../hooks/useUsageState', () => ({ useUsageState: mockUseUsageState }));

vi.mock('../../store/socketSelectors', () => ({
  selectSocketStatus: (state: { socket?: { byUser?: Record<string, { status: string }> } }) =>
    state.socket?.byUser?.__pending__?.status ?? 'disconnected',
}));

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

function socketState(status: 'connected' | 'disconnected') {
  return {
    byUser: { __pending__: { status, socketId: status === 'connected' ? 'socket-1' : null } },
  };
}

function makeFile(name: string, type: string, size = 1024): File {
  const blob = new Blob([new Uint8Array(size)], { type });
  return new File([blob], name, { type });
}

async function renderWithSelectedThread() {
  const thread = makeThread({ id: 'attach-thread', title: 'Attach Thread' });
  mockGetThreads.mockResolvedValue({ threads: [thread], count: 1 });
  mockGetThreadMessages.mockResolvedValue({ messages: [], count: 0 });

  const store = buildStore({
    thread: {
      threads: [thread],
      selectedThreadId: thread.id,
      activeThreadId: null,
      welcomeThreadId: null,
      messagesByThreadId: { [thread.id]: [] },
      messages: [],
      isLoadingThreads: false,
      isLoadingMessages: false,
      messagesError: null,
    },
    socket: socketState('connected'),
  });

  const { default: Conversations } = await import('../Conversations');

  render(
    <Provider store={store}>
      <MemoryRouter initialEntries={['/conversations']}>
        <Conversations />
      </MemoryRouter>
    </Provider>
  );

  const textarea = await screen.findByPlaceholderText('Type a message...');
  return { store, textarea, thread };
}

// ── Tests ────────────────────────────────────────────────────────────────────

describe('Conversations — attachment feature', () => {
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

  it('renders the paperclip button in the composer', async () => {
    await renderWithSelectedThread();
    expect(screen.getByTitle('Attach image')).toBeInTheDocument();
  });

  it('shows attachment chip after selecting a valid image file', async () => {
    await renderWithSelectedThread();

    const fileInput = document.querySelector('input[type="file"]') as HTMLInputElement;
    expect(fileInput).not.toBeNull();

    const file = makeFile('photo.png', 'image/png', 512);
    await act(async () => {
      fireEvent.change(fileInput, { target: { files: [file] } });
    });

    await waitFor(() => {
      expect(screen.getByText('photo.png')).toBeInTheDocument();
    });
  });

  it('shows too-many error when selecting more than 4 images', async () => {
    await renderWithSelectedThread();

    const fileInput = document.querySelector('input[type="file"]') as HTMLInputElement;
    const files = Array.from({ length: 5 }, (_, i) => makeFile(`img${i}.png`, 'image/png', 512));

    await act(async () => {
      fireEvent.change(fileInput, { target: { files } });
    });

    await waitFor(() => {
      expect(screen.getByText(/Maximum 4 images/i)).toBeInTheDocument();
    });
  });

  it('shows unsupported type error for non-image files', async () => {
    await renderWithSelectedThread();

    const fileInput = document.querySelector('input[type="file"]') as HTMLInputElement;
    const file = makeFile('doc.pdf', 'application/pdf', 512);

    await act(async () => {
      fireEvent.change(fileInput, { target: { files: [file] } });
    });

    await waitFor(() => {
      expect(screen.getByText(/Unsupported file type/i)).toBeInTheDocument();
    });
  });

  it('shows too-large error for oversized files', async () => {
    await renderWithSelectedThread();

    const fileInput = document.querySelector('input[type="file"]') as HTMLInputElement;
    const file = makeFile('big.png', 'image/png', 8 * 1024 * 1024 + 1);

    await act(async () => {
      fireEvent.change(fileInput, { target: { files: [file] } });
    });

    await waitFor(() => {
      expect(screen.getByText(/8 MB/i)).toBeInTheDocument();
    });
  });

  it('removes chip when × button is clicked', async () => {
    await renderWithSelectedThread();

    const fileInput = document.querySelector('input[type="file"]') as HTMLInputElement;
    const file = makeFile('remove-me.png', 'image/png', 512);

    await act(async () => {
      fireEvent.change(fileInput, { target: { files: [file] } });
    });

    await waitFor(() => {
      expect(screen.getByText('remove-me.png')).toBeInTheDocument();
    });

    const removeBtn = screen.getByRole('button', { name: /remove remove-me\.png/i });
    await act(async () => {
      fireEvent.click(removeBtn);
    });

    await waitFor(() => {
      expect(screen.queryByText('remove-me.png')).not.toBeInTheDocument();
    });
  });

  it('enables send button when attachment is present with no text', async () => {
    await renderWithSelectedThread();

    const fileInput = document.querySelector('input[type="file"]') as HTMLInputElement;
    const file = makeFile('only.png', 'image/png', 512);

    await act(async () => {
      fireEvent.change(fileInput, { target: { files: [file] } });
    });

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Send message' })).not.toBeDisabled();
    });
  });

  it('clears attachments and calls chatSend after sending with attachment + text', async () => {
    const { chatSend } = await import('../../services/chatService');
    const { textarea } = await renderWithSelectedThread();

    const fileInput = document.querySelector('input[type="file"]') as HTMLInputElement;
    const file = makeFile('send.png', 'image/png', 512);

    await act(async () => {
      fireEvent.change(fileInput, { target: { files: [file] } });
    });

    await waitFor(() => {
      expect(screen.getByText('send.png')).toBeInTheDocument();
    });

    await act(async () => {
      fireEvent.change(textarea, { target: { value: 'describe this' } });
    });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Send message' }));
    });

    await waitFor(() => {
      expect(chatSend).toHaveBeenCalled();
      expect(screen.queryByText('send.png')).not.toBeInTheDocument();
    });
  });

  it('renders image thumbnails in user message bubble from extraMetadata', async () => {
    const thread = makeThread({ id: 'img-thread', title: 'Img Thread' });
    const dataUri = 'data:image/png;base64,abc123';
    const message = {
      id: 'msg-1',
      content: 'look at this',
      type: 'text' as const,
      sender: 'user' as const,
      createdAt: new Date().toISOString(),
      extraMetadata: { attachmentDataUris: [dataUri] },
    };

    mockGetThreads.mockResolvedValue({ threads: [thread], count: 1 });
    mockGetThreadMessages.mockResolvedValue({ messages: [message], count: 1 });

    const store = buildStore({
      thread: {
        threads: [thread],
        selectedThreadId: thread.id,
        activeThreadId: null,
        welcomeThreadId: null,
        messagesByThreadId: { [thread.id]: [message] },
        messages: [message],
        isLoadingThreads: false,
        isLoadingMessages: false,
        messagesError: null,
      },
      socket: socketState('connected'),
    });

    const { default: Conversations } = await import('../Conversations');

    render(
      <Provider store={store}>
        <MemoryRouter>
          <Conversations />
        </MemoryRouter>
      </Provider>
    );

    await waitFor(() => {
      const img = document.querySelector(`img[src="${dataUri}"]`);
      expect(img).not.toBeNull();
    });
  });
});
