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
import { threadApi } from '../../services/api/threadApi';
import { chatSend } from '../../services/chatService';
import { callCoreRpc } from '../../services/coreRpcClient';
import chatRuntimeReducer from '../../store/chatRuntimeSlice';
import layoutReducer from '../../store/layoutSlice';
import queueReducer from '../../store/queueSlice';
import runModeReducer from '../../store/runModeSlice';
import socketReducer from '../../store/socketSlice';
import themeReducer from '../../store/themeSlice';
import threadGoalReducer from '../../store/threadGoalSlice';
import threadReducer, { markThreadInferenceActive } from '../../store/threadSlice';
import threadTodosReducer from '../../store/threadTodosSlice';
import type { Thread } from '../../types/thread';
import Conversations from './Conversations';

vi.mock('../../services/socketService', () => ({
  socketService: {
    getSocket: vi.fn(() => ({ id: 'composer-test-socket' })),
    on: vi.fn(),
    off: vi.fn(),
  },
}));

const {
  mockGetThreads,
  mockGetThreadMessages,
  mockUseUsageState,
  mockChatSend,
  mockChatSendWithAck,
} = vi.hoisted(() => ({
  mockGetThreads: vi.fn().mockResolvedValue({ threads: [], count: 0 }),
  mockGetThreadMessages: vi.fn().mockResolvedValue({ messages: [], count: 0 }),
  mockChatSend: vi.fn().mockResolvedValue(undefined),
  mockChatSendWithAck: vi
    .fn()
    .mockResolvedValue({ requestId: 'queued-request', queuedAttachments: [] }),
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

vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn().mockResolvedValue({}) }));

vi.mock('../../services/chatService', () => ({
  chatCancel: vi.fn().mockResolvedValue({ accepted: true, turnCancelled: true }),
  chatClearQueue: vi.fn().mockResolvedValue(0),
  chatSend: mockChatSend,
  chatSendWithAck: mockChatSendWithAck,
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
      source: { kind: 'cloud'; providerSlug: string } | { kind: 'local' } | { kind: 'managed' };
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
      <button
        type="button"
        onClick={() =>
          onSelect({ source: { kind: 'cloud', providerSlug: 'unknown-provider' }, model: 'model' })
        }>
        Pick unknown provider model
      </button>
      <button
        type="button"
        onClick={() => onSelect({ source: { kind: 'managed' }, model: 'openrouter/author/model' })}>
        Pick managed model
      </button>
      <button
        type="button"
        onClick={() => onSelect({ source: { kind: 'local' }, model: 'qwen3:4b-instruct' })}>
        Pick local model
      </button>
      <button type="button" onClick={() => onSelect({ source: { kind: 'managed' }, model: '' })}>
        Clear model
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
      queue: queueReducer,
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
  return store;
}

async function submitComposerText(text: string) {
  const previousSends = mockChatSend.mock.calls.length + mockChatSendWithAck.mock.calls.length;
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
  await waitFor(() =>
    expect(mockChatSend.mock.calls.length + mockChatSendWithAck.mock.calls.length).toBeGreaterThan(
      previousSends
    )
  );
}

async function clickSendButtonWithDraft(text: string) {
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
}

async function selectPickerModel() {
  fireEvent.click(screen.getByTestId('composer-chat-settings'));
  fireEvent.click(await screen.findByRole('button', { name: 'Pick Hugging Face model' }));
}

async function selectPickerRoute(name: 'managed' | 'local' | 'unknown provider') {
  fireEvent.click(screen.getByTestId('composer-chat-settings'));
  fireEvent.click(await screen.findByRole('button', { name: `Pick ${name} model` }));
}

async function clearPickerModel() {
  fireEvent.click(screen.getByTestId('composer-chat-settings'));
  fireEvent.click(await screen.findByRole('button', { name: 'Clear model' }));
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function mockModelSettingsWrites(writes: Record<string, Promise<unknown> | Promise<unknown>[]>) {
  vi.mocked(callCoreRpc).mockImplementation(({ method, params }) => {
    if (method === 'openhuman.inference_update_model_settings') {
      const defaultModel = (params as { default_model: string }).default_model;
      const writesForValue = writes[defaultModel];
      const write = Array.isArray(writesForValue) ? writesForValue.shift() : writesForValue;
      return (write ?? Promise.resolve({})) as ReturnType<typeof callCoreRpc>;
    }
    return Promise.resolve({}) as ReturnType<typeof callCoreRpc>;
  });
}

async function useRealChatSend() {
  const actual = await vi.importActual<typeof import('../../services/chatService')>(
    '../../services/chatService'
  );
  mockChatSend.mockImplementation(actual.chatSend);
  mockChatSendWithAck.mockImplementation(actual.chatSendWithAck);
}

function latestChatRpc() {
  return vi
    .mocked(callCoreRpc)
    .mock.calls.filter(([request]) => request.method === 'openhuman.channel_web_chat')
    .at(-1)?.[0];
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

  it('persists raw uploads once and sends the core returned opaque reference', async () => {
    const persistedAttachment = {
      kind: 'image',
      media: {
        kind: 'image',
        codec: null,
        opaqueId: 'stored-photo',
        contentType: 'image/png',
        fileName: 'photo.png',
        sizeBytes: 3,
        durationMs: null,
        height: null,
        width: null,
        localCachePath: null,
        path: null,
        platformMessageId: null,
        transcribed: false,
        url: null,
      },
      caption: null,
      altText: null,
    } as const;
    vi.mocked(threadApi.appendMessage).mockImplementationOnce(async (_id, message) => ({
      ...message,
      id: 'persisted-user-row',
      attachments: [persistedAttachment],
    }));
    await renderChat('text');
    const picker = document.querySelector('input[type="file"]');
    expect(picker).not.toBeNull();
    fireEvent.change(picker!, {
      target: { files: [new File([Uint8Array.of(1, 2, 3)], 'photo.png', { type: 'image/png' })] },
    });
    await waitFor(() => expect(screen.getByText('photo.png')).toBeInTheDocument());
    fireEvent.click(screen.getByTestId('send-message-button'));
    await waitFor(() => expect(chatSend).toHaveBeenCalled());
    const [, uploaded, uploadedParts] = vi.mocked(threadApi.appendMessage).mock.calls.at(-1)!;
    // The original rides as a typed part; the text carries no marker.
    expect(uploaded.content).not.toContain('[IMAGE:');
    expect(uploadedParts).toEqual([
      expect.objectContaining({
        kind: 'image',
        media: expect.objectContaining({
          url: 'data:image/png;name=photo.png;base64,AQID',
          fileName: 'photo.png',
        }),
      }),
    ]);
    expect(JSON.stringify(uploaded.extraMetadata)).not.toContain('base64');
    expect(uploaded.extraMetadata).not.toHaveProperty('attachmentDataUris');
    expect(chatSend).toHaveBeenLastCalledWith(
      expect.objectContaining({
        threadId: THREAD_ID,
        message: uploaded.content,
        persistedMessageId: 'persisted-user-row',
        attachments: [persistedAttachment],
      })
    );
    expect(uploaded.content).not.toMatch(/\[(?:ATTACHMENT|IMAGE|FILE):/i);
  });

  it('keeps only core-returned opaque refs for a queued upload', async () => {
    vi.mocked(threadApi.appendMessage).mockClear();
    vi.mocked(chatSend).mockClear();
    mockChatSendWithAck.mockResolvedValueOnce({
      requestId: 'queued-upload-request',
      queuedAttachments: [
        {
          kind: 'file',
          media: {
            opaqueId: 'opaque-queued-file',
            contentType: 'application/zip',
            fileName: 'archive.zip',
            sizeBytes: 3,
          },
          fileName: 'archive.zip',
          caption: null,
        },
      ],
    });
    const store = await renderChat('text');
    fireEvent.change(document.querySelector('input[type="file"]')!, {
      target: {
        files: [new File([Uint8Array.of(1, 2, 3)], 'archive.zip', { type: 'application/zip' })],
      },
    });
    await waitFor(() => expect(screen.getByText('archive.zip')).toBeInTheDocument());
    await act(async () => {
      store.dispatch(markThreadInferenceActive(THREAD_ID));
    });
    fireEvent.click(screen.getByTestId('send-message-button'));
    await waitFor(() => expect(mockChatSendWithAck).toHaveBeenCalled());
    expect(mockChatSendWithAck).toHaveBeenLastCalledWith(
      expect.objectContaining({
        queueMode: 'followup',
        message: '',
        attachments: [
          expect.objectContaining({
            kind: 'file',
            media: expect.objectContaining({
              url: 'data:application/zip;name=archive.zip;base64,AQID',
            }),
          }),
        ],
      })
    );
    expect(threadApi.appendMessage).not.toHaveBeenCalled();
    const queued = store.getState().queue.pendingFollowupsByThread[THREAD_ID][0];
    const pending = queued.message;
    expect(queued.attachments).toHaveLength(1);
    expect(queued.requestId).toBe('queued-upload-request');
    expect(queued.attachments[0]).toMatchObject({
      kind: 'file',
      media: { opaqueId: 'opaque-queued-file' },
    });
    expect(JSON.stringify(queued.attachments)).not.toMatch(/data:|base64|"url"\s*:|"path"\s*:/i);
    expect(pending.content).toBe('');
    expect(pending.extraMetadata).toEqual({
      attachmentCount: 1,
      attachmentNames: ['archive.zip'],
      attachmentKinds: ['file'],
      attachmentCompressed: [false],
    });
    expect(JSON.stringify(pending.extraMetadata)).not.toMatch(/data:|base64/i);
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
    vi.mocked(threadApi.appendMessage)
      .mockReset()
      .mockImplementation(async (_threadId: string, message) => message);
    vi.mocked(callCoreRpc).mockReset().mockResolvedValue({});
    mockChatSend.mockReset().mockResolvedValue(undefined);
    mockChatSendWithAck
      .mockReset()
      .mockResolvedValue({ requestId: 'queued-request', queuedAttachments: [] });
  });

  it('a late client-config read does not restore a model the user already cleared', async () => {
    const config = deferred<unknown>();
    vi.mocked(callCoreRpc).mockImplementation((({ method }: { method: string }) => {
      if (method === 'openhuman.inference_get_client_config') return config.promise;
      if (method === 'openhuman.inference_resolve_model') return Promise.resolve({ model: 'm' });
      return Promise.resolve({});
    }) as never);
    await renderChat('text');
    await selectPickerModel();
    await clearPickerModel();

    await act(async () => config.resolve({ result: { default_model: 'stale-default' } }));

    const hints = vi
      .mocked(callCoreRpc)
      .mock.calls.filter(([req]) => req.method === 'openhuman.inference_resolve_model')
      .map(([req]) => (req.params as { hint: string }).hint);
    expect(hints).not.toContain('stale-default');
  });

  it('waits for a successful model clear before sending a normal default turn', async () => {
    const clear = deferred<unknown>();
    mockModelSettingsWrites({ 'huggingface:org/model': Promise.resolve({}), '': clear.promise });
    await renderChat('text');
    await selectPickerModel();
    await clearPickerModel();

    await clickSendButtonWithDraft('wait for clear');
    expect(mockChatSend).not.toHaveBeenCalled();
    expect(threadApi.appendMessage).not.toHaveBeenCalled();

    await act(async () => clear.resolve({}));
    await waitFor(() => expect(mockChatSend).toHaveBeenCalledTimes(1));
    expect(mockChatSend.mock.calls[0][0]).not.toHaveProperty('model');
  });

  it('waits for a successful model clear before sending a follow-up', async () => {
    const clear = deferred<unknown>();
    mockModelSettingsWrites({ 'huggingface:org/model': Promise.resolve({}), '': clear.promise });
    await renderChat('text', false, true);
    await selectPickerModel();
    await clearPickerModel();

    await clickSendButtonWithDraft('wait for follow-up clear');
    expect(mockChatSend).not.toHaveBeenCalled();

    await act(async () => clear.resolve({}));
    await waitFor(() => expect(mockChatSendWithAck).toHaveBeenCalledTimes(1));
    expect(mockChatSendWithAck.mock.calls[0][0]).toMatchObject({ queueMode: 'followup' });
    expect(mockChatSendWithAck.mock.calls[0][0]).not.toHaveProperty('model');
  });

  it('blocks a normal send after a rejected clear and keeps the draft', async () => {
    const clear = deferred<unknown>();
    mockModelSettingsWrites({ 'huggingface:org/model': Promise.resolve({}), '': clear.promise });
    await renderChat('text');
    await selectPickerModel();
    await clearPickerModel();

    await clickSendButtonWithDraft('retry this draft');
    await act(async () => clear.reject(new Error('clear failed')));

    expect(mockChatSend).not.toHaveBeenCalled();
    expect(threadApi.appendMessage).not.toHaveBeenCalled();
    expect(screen.getByRole('textbox')).toHaveTextContent('retry this draft');
    await waitFor(() =>
      expect(screen.getByTestId('chat-send-error')).toHaveAttribute(
        'data-chat-send-error-code',
        'cloud_send_failed'
      )
    );

    const input = screen.getByRole('textbox');
    await act(async () => {
      input.textContent = 'retry this draft edited';
      fireEvent.input(input, { data: 'retry this draft edited', inputType: 'insertText' });
    });
    await waitFor(() => expect(screen.queryByTestId('chat-send-error')).not.toBeInTheDocument());
    expect(mockChatSend).not.toHaveBeenCalled();
  });

  it('blocks a follow-up after a rejected clear and keeps the draft', async () => {
    const clear = deferred<unknown>();
    mockModelSettingsWrites({ 'huggingface:org/model': Promise.resolve({}), '': clear.promise });
    await renderChat('text', false, true);
    await selectPickerModel();
    await clearPickerModel();

    await clickSendButtonWithDraft('retry this follow-up');
    await act(async () => clear.reject(new Error('clear failed')));

    expect(mockChatSend).not.toHaveBeenCalled();
    expect(screen.getByRole('textbox')).toHaveTextContent('retry this follow-up');
    await waitFor(() =>
      expect(screen.getByTestId('chat-send-error')).toHaveAttribute(
        'data-chat-send-error-code',
        'cloud_send_failed'
      )
    );

    const input = screen.getByRole('textbox');
    await act(async () => {
      input.textContent = 'retry this follow-up edited';
      fireEvent.input(input, { data: 'retry this follow-up edited', inputType: 'insertText' });
    });
    await waitFor(() => expect(screen.queryByTestId('chat-send-error')).not.toBeInTheDocument());
    expect(mockChatSend).not.toHaveBeenCalled();
  });

  it.each([
    { label: 'normal send', followup: false },
    { label: 'follow-up send', followup: true },
  ])(
    'retries a rejected clear on later default $label attempts until one succeeds',
    async ({ followup }) => {
      const firstClear = deferred<unknown>();
      const secondClear = deferred<unknown>();
      const thirdClear = deferred<unknown>();
      mockModelSettingsWrites({
        'huggingface:org/model': Promise.resolve({}),
        '': [firstClear.promise, secondClear.promise, thirdClear.promise],
      });
      await renderChat('text', false, followup);
      await selectPickerModel();
      await clearPickerModel();

      const clearCalls = () =>
        vi.mocked(callCoreRpc).mock.calls.filter(([request]) => {
          const params = request.params as { default_model?: string };
          return (
            request.method === 'openhuman.inference_update_model_settings' &&
            params.default_model === ''
          );
        });

      await clickSendButtonWithDraft('first blocked attempt');
      expect(clearCalls()).toHaveLength(1);
      await act(async () => firstClear.reject(new Error('clear unavailable')));
      expect(mockChatSend).not.toHaveBeenCalled();
      expect(screen.getByRole('textbox')).toHaveTextContent('first blocked attempt');

      await clickSendButtonWithDraft('second blocked attempt');
      await waitFor(() => expect(clearCalls()).toHaveLength(2));
      expect(mockChatSend).not.toHaveBeenCalled();
      expect(threadApi.appendMessage).not.toHaveBeenCalled();
      await act(async () => secondClear.reject(new Error('clear still unavailable')));
      expect(mockChatSend).not.toHaveBeenCalled();
      expect(screen.getByRole('textbox')).toHaveTextContent('second blocked attempt');

      await clickSendButtonWithDraft('third attempt');
      await waitFor(() => expect(clearCalls()).toHaveLength(3));
      expect(mockChatSend).not.toHaveBeenCalled();
      expect(threadApi.appendMessage).not.toHaveBeenCalled();
      await act(async () => thirdClear.resolve({}));

      await waitFor(() =>
        expect(followup ? mockChatSendWithAck : mockChatSend).toHaveBeenCalledTimes(1)
      );
      const sent = followup ? mockChatSendWithAck.mock.calls[0][0] : mockChatSend.mock.calls[0][0];
      expect(sent).not.toHaveProperty('model');
      if (followup) {
        expect(sent).toMatchObject({ queueMode: 'followup' });
      } else {
        expect(sent).not.toHaveProperty('queueMode');
      }
    }
  );

  it('waits on the newer clear when an older clear fails late', async () => {
    const olderClear = deferred<unknown>();
    const newerClear = deferred<unknown>();
    mockModelSettingsWrites({
      'huggingface:org/model': Promise.resolve({}),
      '': [olderClear.promise, newerClear.promise],
    });
    await renderChat('text');
    await selectPickerModel();
    await clearPickerModel();
    await clearPickerModel();

    const clearCalls = () =>
      vi.mocked(callCoreRpc).mock.calls.filter(([request]) => {
        const params = request.params as { default_model?: string };
        return (
          request.method === 'openhuman.inference_update_model_settings' &&
          params.default_model === ''
        );
      });

    await clickSendButtonWithDraft('wait for latest clear');
    expect(clearCalls()).toHaveLength(1);
    expect(mockChatSend).not.toHaveBeenCalled();

    await act(async () => olderClear.reject(new Error('older clear failed late')));
    await waitFor(() => expect(clearCalls()).toHaveLength(2));
    expect(mockChatSend).not.toHaveBeenCalled();

    await act(async () => newerClear.resolve({}));
    await waitFor(() => expect(mockChatSend).toHaveBeenCalledTimes(1));
    expect(mockChatSend.mock.calls[0][0]).not.toHaveProperty('model');
  });

  it('does not wait for selected-model persistence before an explicit-model send', async () => {
    const persist = deferred<unknown>();
    mockModelSettingsWrites({ 'huggingface:org/model': persist.promise });
    await renderChat('text');

    await selectPickerModel();
    await submitComposerText('explicit while persistence is pending');

    expect(mockChatSend.mock.calls[0][0]).toMatchObject({ model: 'huggingface:org/model' });
    expect(persist.promise).toBeInstanceOf(Promise);
    await act(async () => persist.resolve({}));
  });

  it('lets a newly selected explicit model send after an earlier clear fails', async () => {
    const clear = deferred<unknown>();
    mockModelSettingsWrites({ 'huggingface:org/model': Promise.resolve({}), '': clear.promise });
    await renderChat('text');
    await selectPickerModel();
    await clearPickerModel();
    await clickSendButtonWithDraft('blocked default draft');
    await act(async () => clear.reject(new Error('clear failed')));
    expect(mockChatSend).not.toHaveBeenCalled();

    await selectPickerModel();
    await submitComposerText('new explicit route');

    expect(mockChatSend.mock.calls[0][0]).toMatchObject({ model: 'huggingface:org/model' });
  });

  it('serializes a pending selection before a later clear', async () => {
    const selection = deferred<unknown>();
    const clear = deferred<unknown>();
    mockModelSettingsWrites({ 'huggingface:org/model': selection.promise, '': clear.promise });
    await renderChat('text');
    await selectPickerModel();
    await clearPickerModel();

    await clickSendButtonWithDraft('clear wins');
    expect(mockChatSend).not.toHaveBeenCalled();

    await act(async () => selection.resolve({}));
    expect(mockChatSend).not.toHaveBeenCalled();
    await act(async () => clear.resolve({}));
    await waitFor(() => expect(mockChatSend).toHaveBeenCalledTimes(1));
    expect(mockChatSend.mock.calls[0][0]).not.toHaveProperty('model');
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

    expect(mockChatSendWithAck.mock.calls[0][0]).toMatchObject({ queueMode: 'followup' });
    expect(mockChatSendWithAck.mock.calls[0][0]).not.toHaveProperty('model');
  });

  it('forwards the concrete provider/model chosen in the picker for a normal send', async () => {
    mockChatSend.mockClear();
    await useRealChatSend();
    await renderChat('text');

    await selectPickerModel();
    await submitComposerText('explicit picker route');

    expect(latestChatRpc()).toMatchObject({
      method: 'openhuman.channel_web_chat',
      params: { model_override: 'huggingface:org/model' },
    });
    const resolverCalls = vi
      .mocked(callCoreRpc)
      .mock.calls.filter(([request]) => request.method === 'openhuman.inference_resolve_model');
    expect(resolverCalls.at(-1)?.[0]).toMatchObject({
      method: 'openhuman.inference_resolve_model',
      params: { hint: 'huggingface:org/model' },
    });
  });

  it('forwards the concrete provider/model chosen in the picker for a follow-up send', async () => {
    mockChatSend.mockClear();
    await useRealChatSend();
    await renderChat('text', false, true);

    await selectPickerModel();
    await submitComposerText('explicit follow-up picker route');

    expect(latestChatRpc()).toMatchObject({
      method: 'openhuman.channel_web_chat',
      params: { model_override: 'huggingface:org/model', queue_mode: 'followup' },
    });
  });

  it.each([
    ['managed', 'openrouter/author/model'],
    ['local', 'ollama:qwen3:4b-instruct'],
  ] as const)('serializes the %s picker route at the core boundary', async (route, model) => {
    mockChatSend.mockClear();
    await useRealChatSend();
    await renderChat('text');

    await selectPickerRoute(route);
    await submitComposerText(`${route} picker route`);

    expect(latestChatRpc()).toMatchObject({
      method: 'openhuman.channel_web_chat',
      params: { model_override: model },
    });
  });

  it('keeps an unknown provider route rejected at the core boundary', async () => {
    vi.mocked(callCoreRpc).mockImplementation(({ method, params }) => {
      if (
        method === 'openhuman.channel_web_chat' &&
        (params as { model_override?: string }).model_override === 'unknown-provider:model'
      ) {
        return Promise.reject(new Error('unsupported provider route')) as ReturnType<
          typeof callCoreRpc
        >;
      }
      return Promise.resolve({}) as ReturnType<typeof callCoreRpc>;
    });
    await useRealChatSend();
    await renderChat('text');

    await selectPickerRoute('unknown provider');
    await clickSendButtonWithDraft('unknown provider route');

    await waitFor(() => expect(mockChatSend).toHaveBeenCalledTimes(1));
    expect(latestChatRpc()).toMatchObject({
      method: 'openhuman.channel_web_chat',
      params: { model_override: 'unknown-provider:model' },
    });
    await waitFor(() => {
      expect(screen.getByRole('textbox')).toHaveTextContent('unknown provider route');
      expect(screen.getByTestId('chat-send-error')).toHaveAttribute(
        'data-chat-send-error-code',
        'cloud_send_failed'
      );
    });
  });
});
