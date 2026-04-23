import { configureStore } from '@reduxjs/toolkit';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { threadApi } from '../../services/api/threadApi';
import type { Thread, ThreadMessage } from '../../types/thread';
import threadReducer, {
  addInferenceResponse,
  clearAllThreads,
  clearSelectedThread,
  loadThreadMessages,
  loadThreads,
  setActiveThread,
  setSelectedThread,
} from '../threadSlice';

vi.mock('../../services/api/threadApi', () => ({
  threadApi: {
    createNewThread: vi.fn(),
    getThreads: vi.fn(),
    getThreadMessages: vi.fn(),
    appendMessage: vi.fn(),
    deleteThread: vi.fn(),
    generateTitleIfNeeded: vi.fn(),
    updateMessage: vi.fn(),
    purge: vi.fn(),
  },
}));

const mockedThreadApi = vi.mocked(threadApi);

function createStore() {
  return configureStore({ reducer: { thread: threadReducer } });
}

function makeThread(overrides: Partial<Thread> = {}): Thread {
  return {
    id: 't-1',
    title: 'Untitled',
    chatId: null,
    isActive: false,
    messageCount: 0,
    lastMessageAt: '2026-01-01T00:00:00.000Z',
    createdAt: '2026-01-01T00:00:00.000Z',
    ...overrides,
  };
}

function makeMessage(overrides: Partial<ThreadMessage> = {}): ThreadMessage {
  return {
    id: 'm-1',
    content: 'hello',
    type: 'text',
    extraMetadata: {},
    sender: 'user',
    createdAt: '2026-01-01T00:00:00.000Z',
    ...overrides,
  };
}

describe('threadSlice synchronous reducers', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('starts with the expected initial state', () => {
    const store = createStore();
    const state = store.getState().thread;
    expect(state.threads).toEqual([]);
    expect(state.selectedThreadId).toBeNull();
    expect(state.activeThreadId).toBeNull();
    expect(state.messagesByThreadId).toEqual({});
    expect(state.messages).toEqual([]);
    expect(state.isLoadingThreads).toBe(false);
    expect(state.isLoadingMessages).toBe(false);
    expect(state.suggestedQuestions).toEqual([]);
  });

  it('setSelectedThread copies cached messages into the visible list', async () => {
    const store = createStore();
    const cached = [makeMessage({ id: 'm-1' }), makeMessage({ id: 'm-2' })];

    mockedThreadApi.getThreadMessages.mockResolvedValueOnce({
      messages: cached,
      count: cached.length,
    });
    await store.dispatch(loadThreadMessages('t-1'));

    store.dispatch(setSelectedThread('t-1'));
    const state = store.getState().thread;
    expect(state.selectedThreadId).toBe('t-1');
    expect(state.messages).toEqual(cached);
    expect(state.messagesError).toBeNull();
    expect(state.suggestedQuestions).toEqual([]);
  });

  it('setSelectedThread resets messages when cache is empty', () => {
    const store = createStore();
    store.dispatch(setSelectedThread('missing'));
    const state = store.getState().thread;
    expect(state.selectedThreadId).toBe('missing');
    expect(state.messages).toEqual([]);
  });

  it('clearSelectedThread clears visible selection but keeps cache', async () => {
    const store = createStore();
    mockedThreadApi.getThreadMessages.mockResolvedValueOnce({
      messages: [makeMessage()],
      count: 1,
    });
    await store.dispatch(loadThreadMessages('t-1'));
    store.dispatch(setSelectedThread('t-1'));

    store.dispatch(clearSelectedThread());
    const state = store.getState().thread;
    expect(state.selectedThreadId).toBeNull();
    expect(state.messages).toEqual([]);
    expect(state.messagesByThreadId['t-1']).toHaveLength(1);
  });

  it('setActiveThread only touches the active id', () => {
    const store = createStore();
    store.dispatch(setActiveThread('t-active'));
    expect(store.getState().thread.activeThreadId).toBe('t-active');
    store.dispatch(setActiveThread(null));
    expect(store.getState().thread.activeThreadId).toBeNull();
  });

  it('clearAllThreads wipes threads, messages, and selection', async () => {
    const store = createStore();
    mockedThreadApi.getThreads.mockResolvedValueOnce({
      threads: [makeThread({ id: 't-1' })],
      count: 1,
    });
    await store.dispatch(loadThreads());
    mockedThreadApi.getThreadMessages.mockResolvedValueOnce({
      messages: [makeMessage()],
      count: 1,
    });
    await store.dispatch(loadThreadMessages('t-1'));
    store.dispatch(setSelectedThread('t-1'));
    store.dispatch(setActiveThread('t-1'));

    store.dispatch(clearAllThreads());
    const state = store.getState().thread;
    expect(state.threads).toEqual([]);
    expect(state.messagesByThreadId).toEqual({});
    expect(state.selectedThreadId).toBeNull();
    expect(state.activeThreadId).toBeNull();
    expect(state.messages).toEqual([]);
  });
});

describe('threadSlice loadThreads thunk', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('sets isLoadingThreads while pending and stores threads on fulfilled', async () => {
    const store = createStore();
    const payload = { threads: [makeThread({ id: 'a' })], count: 1 };
    mockedThreadApi.getThreads.mockImplementationOnce(async () => {
      expect(store.getState().thread.isLoadingThreads).toBe(true);
      return payload;
    });

    await store.dispatch(loadThreads());
    const state = store.getState().thread;
    expect(state.isLoadingThreads).toBe(false);
    expect(state.threads).toEqual(payload.threads);
  });

  it('clears loading on rejection', async () => {
    const store = createStore();
    mockedThreadApi.getThreads.mockRejectedValueOnce(new Error('network down'));

    const result = await store.dispatch(loadThreads());
    expect(result.type).toBe('thread/loadThreads/rejected');
    expect(store.getState().thread.isLoadingThreads).toBe(false);
  });
});

describe('threadSlice loadThreadMessages thunk', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('hydrates messagesByThreadId and mirrors visible list when selected', async () => {
    const store = createStore();
    store.dispatch(setSelectedThread('t-1'));
    const messages = [makeMessage({ id: 'a' }), makeMessage({ id: 'b' })];
    mockedThreadApi.getThreadMessages.mockResolvedValueOnce({ messages, count: messages.length });

    await store.dispatch(loadThreadMessages('t-1'));
    const state = store.getState().thread;
    expect(state.messagesByThreadId['t-1']).toEqual(messages);
    expect(state.messages).toEqual(messages);
    expect(state.isLoadingMessages).toBe(false);
    expect(state.messagesError).toBeNull();
  });

  it('does not overwrite visible messages when loading a non-selected thread', async () => {
    const store = createStore();
    mockedThreadApi.getThreadMessages.mockResolvedValueOnce({
      messages: [makeMessage({ id: 'x' })],
      count: 1,
    });
    await store.dispatch(loadThreadMessages('t-1'));
    store.dispatch(setSelectedThread('t-1'));

    mockedThreadApi.getThreadMessages.mockResolvedValueOnce({
      messages: [makeMessage({ id: 'y', content: 'other thread' })],
      count: 1,
    });
    await store.dispatch(loadThreadMessages('t-2'));

    const state = store.getState().thread;
    expect(state.messagesByThreadId['t-2']).toHaveLength(1);
    expect(state.messagesByThreadId['t-2'][0].content).toBe('other thread');
    // Visible messages stayed pinned to t-1.
    expect(state.messages).toHaveLength(1);
    expect(state.messages[0].id).toBe('x');
  });

  it('records messagesError on rejection', async () => {
    const store = createStore();
    mockedThreadApi.getThreadMessages.mockRejectedValueOnce(new Error('boom'));
    await store.dispatch(loadThreadMessages('t-1'));
    const state = store.getState().thread;
    expect(state.isLoadingMessages).toBe(false);
    expect(state.messagesError).toBe('boom');
  });
});

describe('threadSlice addInferenceResponse thunk', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('appends to the supplied thread even when activeThreadId is null', async () => {
    const store = createStore();
    const persisted = makeMessage({ id: 'srv-1', sender: 'agent', content: 'ack' });
    mockedThreadApi.appendMessage.mockResolvedValueOnce(persisted);

    const result = await store.dispatch(addInferenceResponse({ content: 'ack', threadId: 't-1' }));

    expect(result.type).toBe('thread/addInferenceResponse/fulfilled');
    const state = store.getState().thread;
    expect(state.messagesByThreadId['t-1']).toEqual([persisted]);
    // activeThreadId must not be mutated by this thunk — only ChatRuntimeProvider clears it.
    expect(state.activeThreadId).toBeNull();
  });

  it('falls back to activeThreadId when no threadId is supplied', async () => {
    const store = createStore();
    store.dispatch(setActiveThread('t-active'));
    mockedThreadApi.appendMessage.mockResolvedValueOnce(makeMessage({ sender: 'agent' }));

    await store.dispatch(addInferenceResponse({ content: 'ack' }));
    expect(mockedThreadApi.appendMessage).toHaveBeenCalledWith(
      't-active',
      expect.objectContaining({ sender: 'agent', content: 'ack' })
    );
    // activeThreadId must not be cleared by this thunk — ChatRuntimeProvider owns that.
    expect(store.getState().thread.activeThreadId).toBe('t-active');
  });

  it('rejects cleanly when neither threadId nor activeThreadId is set', async () => {
    const store = createStore();
    const result = await store.dispatch(addInferenceResponse({ content: 'ack' }));
    expect(result.type).toBe('thread/addInferenceResponse/rejected');
    expect(mockedThreadApi.appendMessage).not.toHaveBeenCalled();
  });
});
