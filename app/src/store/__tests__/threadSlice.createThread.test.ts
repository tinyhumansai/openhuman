/**
 * #5156 — a `threads_create_new` that blows the 30 s RPC budget must land as a
 * recorded, displayable failure, never as an unhandled rejection.
 *
 * The original Sentry report (TAURI-REACT-10) read `UnhandledRejection:
 * Non-Error promise rejection captured with value: Core RPC
 * openhuman.threads_create_new timed out after 30000ms`. Two things made that
 * possible: `.unwrap()` throws the `rejectWithValue` payload (a bare string with
 * no stack, hence "Non-Error"), and nothing in the store observed
 * `createNewThread.rejected` — so handling was per-call-site, and a site that
 * forgot leaked, while a site that caught-and-ignored showed the user nothing.
 */
import { configureStore } from '@reduxjs/toolkit';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { threadApi } from '../../services/api/threadApi';
import { CoreRpcError } from '../../services/coreRpcClient';
import type { Thread } from '../../types/thread';
import threadReducer, {
  clearCreateThreadError,
  createNewThread,
  formatThreadCreateError,
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
    updateLabels: vi.fn(),
    updateTitle: vi.fn(),
    purge: vi.fn(),
  },
}));

const mockedThreadApi = vi.mocked(threadApi);

const TIMEOUT_MESSAGE = 'Core RPC openhuman.threads_create_new timed out after 30000ms';

function createStore() {
  return configureStore({ reducer: { thread: threadReducer } });
}

function makeThread(overrides: Partial<Thread> = {}): Thread {
  return {
    id: 't-1',
    title: 'Chat Jul 30 1:23 AM',
    chatId: null,
    isActive: false,
    messageCount: 0,
    lastMessageAt: '2026-07-30T00:00:00.000Z',
    createdAt: '2026-07-30T00:00:00.000Z',
    labels: [],
    ...overrides,
  };
}

describe('createNewThread failure handling (#5156)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('records the RPC timeout in state instead of leaving it to the call site', async () => {
    const store = createStore();
    mockedThreadApi.createNewThread.mockRejectedValueOnce(
      new CoreRpcError(TIMEOUT_MESSAGE, 'timeout')
    );

    const result = await store.dispatch(createNewThread(undefined));

    expect(result.type).toBe('thread/createNewThread/rejected');
    expect(store.getState().thread.createThreadError).toBe(TIMEOUT_MESSAGE);
  });

  it('surfaces the timeout to `.unwrap()` callers as a catchable rejection', async () => {
    const store = createStore();
    mockedThreadApi.createNewThread.mockRejectedValueOnce(
      new CoreRpcError(TIMEOUT_MESSAGE, 'timeout')
    );

    const thrown = await store
      .dispatch(createNewThread(undefined))
      .unwrap()
      .then(() => null)
      .catch((err: unknown) => err);

    expect(thrown).not.toBeNull();
    // Whatever shape `.unwrap()` throws, the normaliser yields the message a
    // caller can log or display — that is what keeps both the Sentry breadcrumb
    // and the UI banner readable.
    expect(formatThreadCreateError(thrown)).toBe(TIMEOUT_MESSAGE);
  });

  it('clears the recorded failure when a retry starts and when it succeeds', async () => {
    const store = createStore();
    mockedThreadApi.createNewThread.mockRejectedValueOnce(
      new CoreRpcError(TIMEOUT_MESSAGE, 'timeout')
    );
    await store.dispatch(createNewThread(undefined));
    expect(store.getState().thread.createThreadError).toBe(TIMEOUT_MESSAGE);

    mockedThreadApi.createNewThread.mockResolvedValueOnce(makeThread());
    mockedThreadApi.getThreads.mockResolvedValueOnce({ threads: [makeThread()], count: 1 });
    await store.dispatch(createNewThread(undefined));

    expect(store.getState().thread.createThreadError).toBeNull();
  });

  it('clears the recorded failure on explicit dismissal', async () => {
    const store = createStore();
    mockedThreadApi.createNewThread.mockRejectedValueOnce(
      new CoreRpcError(TIMEOUT_MESSAGE, 'timeout')
    );
    await store.dispatch(createNewThread(undefined));

    store.dispatch(clearCreateThreadError());

    expect(store.getState().thread.createThreadError).toBeNull();
  });

  it('records a failure that comes from the follow-up thread reload', async () => {
    const store = createStore();
    mockedThreadApi.createNewThread.mockResolvedValueOnce(makeThread());
    // `createNewThread` awaits `loadThreads().unwrap()`, which throws a bare
    // string payload rather than an `Error` — the shape that used to fall through
    // the `error instanceof Error` check and be reported as a generic message.
    mockedThreadApi.getThreads.mockRejectedValueOnce(
      new CoreRpcError('Core RPC openhuman.threads_list timed out after 30000ms', 'timeout')
    );

    const result = await store.dispatch(createNewThread(undefined));

    expect(result.type).toBe('thread/createNewThread/rejected');
    expect(store.getState().thread.createThreadError).toBe(
      'Core RPC openhuman.threads_list timed out after 30000ms'
    );
  });

  describe('formatThreadCreateError', () => {
    it('passes a bare string payload through', () => {
      expect(formatThreadCreateError(TIMEOUT_MESSAGE)).toBe(TIMEOUT_MESSAGE);
    });

    it('reads `message` off an Error and off a SerializedError-shaped object', () => {
      expect(formatThreadCreateError(new Error('boom'))).toBe('boom');
      expect(formatThreadCreateError({ name: 'Error', message: 'serialized boom' })).toBe(
        'serialized boom'
      );
    });

    it('falls back for shapes that carry no message', () => {
      expect(formatThreadCreateError(undefined)).toBe('Failed to create thread');
      expect(formatThreadCreateError('   ')).toBe('Failed to create thread');
      expect(formatThreadCreateError({})).toBe('Failed to create thread');
    });

    // A native `new Error()` has `message === ''`. Returning it verbatim stored
    // a falsy `createThreadError`, which `deriveChatErrorBanner` reads as "no
    // failure" — so the user got the dead New Chat button with no banner, the
    // exact outcome #5156 exists to prevent.
    it('falls back for an Error carrying an empty or blank message', () => {
      expect(formatThreadCreateError(new Error())).toBe('Failed to create thread');
      expect(formatThreadCreateError(new Error(''))).toBe('Failed to create thread');
      expect(formatThreadCreateError(new Error('   '))).toBe('Failed to create thread');
      expect(formatThreadCreateError({ name: 'Error', message: '   ' })).toBe(
        'Failed to create thread'
      );
    });
  });

  // Two creates overlap when the user hits New Chat again while the first RPC
  // is still hung on its 30 s budget. Only the latest attempt may write the
  // create-error state, or a late failure from the superseded one repaints the
  // banner over a chat that was actually created.
  describe('overlapping create attempts', () => {
    it('ignores a stale rejection that lands after a newer create succeeded', async () => {
      const store = createStore();

      // Request A hangs; we hold its rejection until after B has succeeded.
      let failA: (reason: unknown) => void = () => {};
      mockedThreadApi.createNewThread.mockImplementationOnce(
        () =>
          new Promise<Thread>((_resolve, reject) => {
            failA = reject;
          })
      );
      const a = store.dispatch(createNewThread(undefined));

      // Request B starts and succeeds while A is still in flight.
      mockedThreadApi.createNewThread.mockResolvedValueOnce(makeThread());
      mockedThreadApi.getThreads.mockResolvedValueOnce({ threads: [makeThread()], count: 1 });
      await store.dispatch(createNewThread(undefined));
      expect(store.getState().thread.createThreadError).toBeNull();

      // Only now does A time out.
      failA(new CoreRpcError(TIMEOUT_MESSAGE, 'timeout'));
      await a;

      // The banner must stay clear — B's chat exists.
      expect(store.getState().thread.createThreadError).toBeNull();
    });

    it('still records the failure when the latest attempt is the one that fails', async () => {
      const store = createStore();

      let succeedA: (thread: Thread) => void = () => {};
      mockedThreadApi.createNewThread.mockImplementationOnce(
        () =>
          new Promise<Thread>(resolve => {
            succeedA = resolve;
          })
      );
      const a = store.dispatch(createNewThread(undefined));

      // B starts after A and fails — B is the live attempt, so its failure counts.
      mockedThreadApi.createNewThread.mockRejectedValueOnce(
        new CoreRpcError(TIMEOUT_MESSAGE, 'timeout')
      );
      await store.dispatch(createNewThread(undefined));
      expect(store.getState().thread.createThreadError).toBe(TIMEOUT_MESSAGE);

      // A's late *success* must not clear a banner it no longer owns.
      mockedThreadApi.getThreads.mockResolvedValueOnce({ threads: [makeThread()], count: 1 });
      succeedA(makeThread());
      await a;

      expect(store.getState().thread.createThreadError).toBe(TIMEOUT_MESSAGE);
    });
  });
});
