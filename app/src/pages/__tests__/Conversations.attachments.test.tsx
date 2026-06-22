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

import { SidebarSlotOutlet, SidebarSlotProvider } from '../../components/layout/shell/SidebarSlot';
import agentProfileReducer from '../../store/agentProfileSlice';
import chatRuntimeReducer from '../../store/chatRuntimeSlice';
import socketReducer from '../../store/socketSlice';
import threadReducer from '../../store/threadSlice';
import type { Thread } from '../../types/thread';

// ── Hoisted mock state ──────────────────────────────────────────────────────

const {
  mockGetThreads,
  mockGetThreadMessages,
  mockSelectAgentProfile,
  mockUseUsageState,
  mockVisionState,
} = vi.hoisted(() => ({
  // Mutable holder so individual tests can flip the resolved model's vision
  // capability without re-mocking the module.
  mockVisionState: { vision: true },
  mockGetThreads: vi.fn().mockResolvedValue({ threads: [], count: 0 }),
  mockGetThreadMessages: vi.fn().mockResolvedValue({ messages: [], count: 0 }),
  mockSelectAgentProfile: vi.fn().mockImplementation((profileId: string) =>
    Promise.resolve({
      activeProfileId: profileId,
      profiles: [
        {
          id: 'default',
          name: 'Default',
          description: 'Default',
          agentId: 'orchestrator',
          builtIn: true,
        },
        {
          id: 'reasoning',
          name: 'Reasoning',
          description: 'Reasoning',
          agentId: 'orchestrator',
          modelOverride: 'hint:reasoning',
          builtIn: true,
        },
      ],
    })
  ),
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
    list: vi.fn().mockResolvedValue({
      activeProfileId: 'default',
      profiles: [
        {
          id: 'default',
          name: 'Default',
          description: 'Default',
          agentId: 'orchestrator',
          builtIn: true,
        },
        {
          id: 'reasoning',
          name: 'Reasoning',
          description: 'Reasoning',
          agentId: 'orchestrator',
          modelOverride: 'hint:reasoning',
          builtIn: true,
        },
      ],
    }),
    select: mockSelectAgentProfile,
    upsert: vi.fn().mockResolvedValue({ activeProfileId: 'default', profiles: [] }),
    delete: vi.fn().mockResolvedValue({ activeProfileId: 'default', profiles: [] }),
  },
}));

vi.mock('../../hooks/useUsageState', () => ({ useUsageState: mockUseUsageState }));

// The new-window hero pulls useUser/useCoreState; stub it (these tests assert
// the composer/attachments, not the empty-state hero).
vi.mock('../../components/chat/ChatNewWindowHero', () => ({ default: () => null }));

vi.mock('../../utils/config', async importActual => ({
  ...(await importActual<typeof import('../../utils/config')>()),
  CHAT_ATTACHMENTS_ENABLED: true,
}));

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

// The composer gates image attachments on the resolved model's vision capability
// (inference_resolve_model). Most tests exercise image attachments, so the
// active model resolves as vision-capable by default; the rejection test flips
// `mockVisionState.vision` to exercise the non-vision path.
vi.mock('../../services/coreRpcClient', () => ({
  callCoreRpc: vi.fn(async ({ method }: { method: string }) =>
    method === 'openhuman.inference_resolve_model'
      ? {
          model: mockVisionState.vision ? 'test-vision-model' : 'reasoning-v1',
          vision: mockVisionState.vision,
        }
      : {}
  ),
  CoreRpcError: class CoreRpcError extends Error {},
}));

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
      activeThreadIds: {},
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
        <SidebarSlotProvider>
          <SidebarSlotOutlet />
          <Conversations />
        </SidebarSlotProvider>
      </MemoryRouter>
    </Provider>
  );

  const textarea = await screen.findByPlaceholderText('How can I help you today?');
  return { store, textarea, thread };
}

// ── Tests ────────────────────────────────────────────────────────────────────

describe('Conversations — attachment feature', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockVisionState.vision = true;
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

  it('renders the attachment button in the composer', async () => {
    await renderWithSelectedThread();
    expect(screen.getByTitle('Attach file')).toBeInTheDocument();
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

  it('shows unsupported type error for unsupported files', async () => {
    await renderWithSelectedThread();

    const fileInput = document.querySelector('input[type="file"]') as HTMLInputElement;
    const file = makeFile('vector.svg', 'image/svg+xml', 512);

    await act(async () => {
      fireEvent.change(fileInput, { target: { files: [file] } });
    });

    await waitFor(() => {
      expect(screen.getByText(/Unsupported file type/i)).toBeInTheDocument();
    });
  });

  it('shows attachment chip after selecting a supported document file', async () => {
    await renderWithSelectedThread();

    const fileInput = document.querySelector('input[type="file"]') as HTMLInputElement;
    const file = makeFile('doc.pdf', 'application/pdf', 512);

    await act(async () => {
      fireEvent.change(fileInput, { target: { files: [file] } });
    });

    await waitFor(() => {
      expect(screen.getByText('doc.pdf')).toBeInTheDocument();
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

  it('keeps the selected profile when an attachment is added (no auto-switch)', async () => {
    await renderWithSelectedThread();

    const fileInput = document.querySelector('input[type="file"]') as HTMLInputElement;
    const file = makeFile('keep-profile.png', 'image/png', 512);

    await act(async () => {
      fireEvent.change(fileInput, { target: { files: [file] } });
    });

    await waitFor(() => {
      expect(screen.getByText('keep-profile.png')).toBeInTheDocument();
    });

    // Adding an attachment must never hijack the user's model selection.
    expect(mockSelectAgentProfile).not.toHaveBeenCalled();
  });

  it('selects the Reasoning tier from the chat-header toggle', async () => {
    await renderWithSelectedThread();

    const reasoningButton = await screen.findByRole('radio', { name: 'Reasoning' });
    expect(reasoningButton).toHaveAttribute('aria-checked', 'false');

    await act(async () => {
      fireEvent.click(reasoningButton);
    });

    await waitFor(() => {
      expect(mockSelectAgentProfile).toHaveBeenCalledWith('reasoning');
      // Store updates to the new active profile → toggle reflects the selection.
      expect(screen.getByRole('radio', { name: 'Reasoning' })).toHaveAttribute(
        'aria-checked',
        'true'
      );
    });
  });

  it('rejects an image and shows the advisory when the model lacks vision', async () => {
    mockVisionState.vision = false;
    await renderWithSelectedThread();

    const fileInput = document.querySelector('input[type="file"]') as HTMLInputElement;
    const file = makeFile('no-vision.png', 'image/png', 512);

    await act(async () => {
      fireEvent.change(fileInput, { target: { files: [file] } });
    });

    // Advisory points users at the vision-capable managed tier.
    await waitFor(() => {
      expect(screen.getByText(/OpenHuman Reasoning tier/i)).toBeInTheDocument();
    });
    // The image is not attached, and the profile is left untouched.
    expect(screen.queryByText('no-vision.png')).not.toBeInTheDocument();
    expect(mockSelectAgentProfile).not.toHaveBeenCalled();
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
      expect(chatSend).toHaveBeenCalledWith(
        expect.objectContaining({
          // Sends with the selected profile's model (default → hint:chat); the
          // attachment no longer forces hint:reasoning.
          model: 'hint:chat',
          message: expect.stringContaining('[IMAGE:data:image/png;base64,'),
        })
      );
      expect(screen.queryByText('send.png')).not.toBeInTheDocument();
    });
  });

  it('sends supported document files as FILE markers through the selected model', async () => {
    const { chatSend } = await import('../../services/chatService');
    const { textarea } = await renderWithSelectedThread();

    const fileInput = document.querySelector('input[type="file"]') as HTMLInputElement;
    const file = makeFile('doc.pdf', 'application/pdf', 512);

    await act(async () => {
      fireEvent.change(fileInput, { target: { files: [file] } });
    });

    await waitFor(() => {
      expect(screen.getByText('doc.pdf')).toBeInTheDocument();
    });

    await act(async () => {
      fireEvent.change(textarea, { target: { value: 'read this' } });
    });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Send message' }));
    });

    await waitFor(() => {
      expect(chatSend).toHaveBeenCalledWith(
        expect.objectContaining({
          // Documents are text-extracted and go through the selected profile's
          // model (default → hint:chat), not a forced reasoning model.
          model: 'hint:chat',
          message: expect.stringContaining('[FILE:data:application/pdf;base64,'),
        })
      );
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
        activeThreadIds: {},
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
          <SidebarSlotProvider>
            <SidebarSlotOutlet />
            <Conversations />
          </SidebarSlotProvider>
        </MemoryRouter>
      </Provider>
    );

    await waitFor(() => {
      const img = document.querySelector(`img[src="${dataUri}"]`);
      expect(img).not.toBeNull();
    });
  });

  it('renders a document filename chip in the user bubble from attachmentKinds/Names', async () => {
    const thread = makeThread({ id: 'file-thread', title: 'File Thread' });
    const message = {
      id: 'msg-file-1',
      content: 'whats in this file',
      type: 'text' as const,
      sender: 'user' as const,
      createdAt: new Date().toISOString(),
      extraMetadata: {
        attachmentCount: 1,
        attachmentKinds: ['file'],
        attachmentNames: ['report.pdf'],
      },
    };

    mockGetThreads.mockResolvedValue({ threads: [thread], count: 1 });
    mockGetThreadMessages.mockResolvedValue({ messages: [message], count: 1 });

    const store = buildStore({
      thread: {
        threads: [thread],
        selectedThreadId: thread.id,
        activeThreadIds: {},
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
          <SidebarSlotProvider>
            <SidebarSlotOutlet />
            <Conversations />
          </SidebarSlotProvider>
        </MemoryRouter>
      </Provider>
    );

    // The document attachment surfaces as a filename chip (not an <img>).
    await waitFor(() => {
      expect(document.body.textContent).toContain('report.pdf');
    });
  });

  it('strips raw IMAGE/FILE markers from a legacy message with no extraMetadata', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    const thread = makeThread({ id: 'legacy-thread', title: 'Legacy Thread' });
    const dataUri = 'data:image/png;base64,legacy123';
    const message = {
      id: 'msg-legacy-1',
      content: `read this [IMAGE:${dataUri}] and [FILE:data:application/pdf;base64,xyz]`,
      type: 'text' as const,
      sender: 'user' as const,
      createdAt: new Date().toISOString(),
      extraMetadata: {},
    };

    mockGetThreads.mockResolvedValue({ threads: [thread], count: 1 });
    mockGetThreadMessages.mockResolvedValue({ messages: [message], count: 1 });

    const store = buildStore({
      thread: {
        threads: [thread],
        selectedThreadId: thread.id,
        activeThreadIds: {},
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
          <SidebarSlotProvider>
            <SidebarSlotOutlet />
            <Conversations />
          </SidebarSlotProvider>
        </MemoryRouter>
      </Provider>
    );

    // The image marker's data URI still renders as an <img> (parsed out for display)...
    await waitFor(() => {
      const img = document.querySelector(`img[src="${dataUri}"]`);
      expect(img).not.toBeNull();
    });

    // ...but the raw marker syntax must never leak into the rendered bubble text.
    expect(document.body.textContent).not.toContain('[IMAGE:');
    expect(document.body.textContent).not.toContain('[FILE:');
    expect(document.body.textContent).toContain('read this');
    expect(document.body.textContent).toContain('and');

    // Copy-to-clipboard must use the same cleaned text as the bubble, not the
    // raw msg.content with markers still embedded.
    await act(async () => {
      fireEvent.click(screen.getByTitle('Copy response'));
    });
    expect(writeText).toHaveBeenCalledWith('read this and');
    expect(writeText).not.toHaveBeenCalledWith(expect.stringContaining('[IMAGE:'));
    expect(writeText).not.toHaveBeenCalledWith(expect.stringContaining('[FILE:'));
  });
});

describe('Conversations — thread rename', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockGetThreads.mockResolvedValue({ threads: [], count: 0 });
    mockGetThreadMessages.mockResolvedValue({ messages: [], count: 0 });
  });

  it('commits an inline thread-title rename from the sidebar thread row', async () => {
    const { thread } = await renderWithSelectedThread();
    const { threadApi } = await import('../../services/api/threadApi');

    // Enter edit mode via the thread row pencil affordance.
    fireEvent.click(screen.getByRole('button', { name: 'Edit thread title' }));
    const input = await screen.findByRole('textbox', { name: 'Edit thread title' });
    fireEvent.change(input, { target: { value: 'Renamed in header' } });
    fireEvent.keyDown(input, { key: 'Enter' });

    await waitFor(() => {
      expect(threadApi.updateTitle).toHaveBeenCalledWith(thread.id, 'Renamed in header');
    });
  });

  it('cancels the rename on Escape without dispatching an update', async () => {
    await renderWithSelectedThread();
    const { threadApi } = await import('../../services/api/threadApi');

    fireEvent.click(screen.getByRole('button', { name: 'Edit thread title' }));
    const input = await screen.findByRole('textbox', { name: 'Edit thread title' });
    fireEvent.change(input, { target: { value: 'Discarded title' } });
    fireEvent.keyDown(input, { key: 'Escape' });

    // Editor closes back to the title heading; no persistence call fired.
    await waitFor(() => {
      expect(screen.queryByRole('textbox', { name: 'Edit thread title' })).toBeNull();
    });
    expect(threadApi.updateTitle).not.toHaveBeenCalled();
  });

  it('does not commit on the Enter that confirms an IME composition', async () => {
    await renderWithSelectedThread();
    const { threadApi } = await import('../../services/api/threadApi');

    fireEvent.click(screen.getByRole('button', { name: 'Edit thread title' }));
    const input = await screen.findByRole('textbox', { name: 'Edit thread title' });
    fireEvent.change(input, { target: { value: '日本語' } });
    // keyCode 229 marks an IME composition keydown — Enter here confirms a
    // candidate, not the rename.
    fireEvent.keyDown(input, { key: 'Enter', keyCode: 229 });

    expect(threadApi.updateTitle).not.toHaveBeenCalled();
    // Editor stays open for continued composition.
    expect(screen.getByRole('textbox', { name: 'Edit thread title' })).toBeInTheDocument();
  });

  it('skips persistence when the committed title is unchanged', async () => {
    await renderWithSelectedThread();
    const { threadApi } = await import('../../services/api/threadApi');

    // The input seeds with the current title ("Attach Thread"); committing it
    // unchanged must not dispatch an update.
    fireEvent.click(screen.getByRole('button', { name: 'Edit thread title' }));
    const input = await screen.findByRole('textbox', { name: 'Edit thread title' });
    fireEvent.keyDown(input, { key: 'Enter' });

    await waitFor(() => {
      expect(screen.queryByRole('textbox', { name: 'Edit thread title' })).toBeNull();
    });
    expect(threadApi.updateTitle).not.toHaveBeenCalled();
  });

  it('skips persistence when the committed title is blank', async () => {
    await renderWithSelectedThread();
    const { threadApi } = await import('../../services/api/threadApi');

    fireEvent.click(screen.getByRole('button', { name: 'Edit thread title' }));
    const input = await screen.findByRole('textbox', { name: 'Edit thread title' });
    fireEvent.change(input, { target: { value: '   ' } });
    fireEvent.keyDown(input, { key: 'Enter' });

    await waitFor(() => {
      expect(screen.queryByRole('textbox', { name: 'Edit thread title' })).toBeNull();
    });
    expect(threadApi.updateTitle).not.toHaveBeenCalled();
  });

  it('opens the editor and ignores the immediate focus-blur', async () => {
    await renderWithSelectedThread();
    const { threadApi } = await import('../../services/api/threadApi');

    // Clicking the row pencil opens edit mode; the blur fired while the input
    // is grabbing focus is ignored (no spurious commit).
    fireEvent.click(screen.getByRole('button', { name: 'Edit thread title' }));
    const input = await screen.findByRole('textbox', { name: 'Edit thread title' });
    fireEvent.blur(input);

    expect(threadApi.updateTitle).not.toHaveBeenCalled();
  });

  it('swallows a rename persistence failure without crashing', async () => {
    const { thread } = await renderWithSelectedThread();
    const { threadApi } = await import('../../services/api/threadApi');
    (threadApi.updateTitle as ReturnType<typeof vi.fn>).mockRejectedValueOnce(
      new Error('rename boom')
    );

    fireEvent.click(screen.getByRole('button', { name: 'Edit thread title' }));
    const input = await screen.findByRole('textbox', { name: 'Edit thread title' });
    fireEvent.change(input, { target: { value: 'Doomed rename' } });
    fireEvent.keyDown(input, { key: 'Enter' });

    await waitFor(() => {
      expect(threadApi.updateTitle).toHaveBeenCalledWith(thread.id, 'Doomed rename');
    });
    // The editor still closes and the UI stays mounted.
    await waitFor(() => {
      expect(screen.queryByRole('textbox', { name: 'Edit thread title' })).toBeNull();
    });
  });
});
