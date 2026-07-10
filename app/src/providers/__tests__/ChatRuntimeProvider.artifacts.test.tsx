import { render } from '@testing-library/react';
import { act } from 'react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import * as chatService from '../../services/chatService';
import { threadApi } from '../../services/api/threadApi';
import { store } from '../../store';
import { clearAllChatRuntime } from '../../store/chatRuntimeSlice';
import { setStatusForUser } from '../../store/socketSlice';
import { clearAllThreads } from '../../store/threadSlice';
import ChatRuntimeProvider from '../ChatRuntimeProvider';

vi.mock('../../services/chatService', async () => {
  const actual = await vi.importActual<typeof chatService>('../../services/chatService');
  return { ...actual, subscribeChatEvents: vi.fn() };
});

vi.mock('../../services/api/threadApi', () => ({
  threadApi: {
    createNewThread: vi.fn(),
    getThreads: vi.fn(),
    getThreadMessages: vi.fn(),
    appendMessage: vi.fn(),
    generateTitleIfNeeded: vi.fn(),
    updateMessage: vi.fn(),
    deleteThread: vi.fn(),
    purge: vi.fn(),
    getTaskBoard: vi.fn(),
    putTaskBoard: vi.fn(),
  },
}));

vi.mock('../../hooks/usageRefresh', () => ({ requestUsageRefresh: vi.fn() }));

const mockRefetchSnapshot = vi.fn();
vi.mock('../../hooks/useRefetchSnapshotOnTurnEnd', () => ({
  useRefetchSnapshotOnTurnEnd: () => ({ refetch: mockRefetchSnapshot }),
}));

function renderProvider(): chatService.ChatEventListeners {
  let captured: chatService.ChatEventListeners = {};
  vi.mocked(chatService.subscribeChatEvents).mockImplementation(listeners => {
    captured = listeners;
    return () => {};
  });

  store.dispatch(setStatusForUser({ userId: '__pending__', status: 'connected' }));

  render(
    <Provider store={store}>
      <ChatRuntimeProvider>
        <div />
      </ChatRuntimeProvider>
    </Provider>
  );

  return captured;
}

function resetRuntimeState() {
  store.dispatch(clearAllThreads());
  store.dispatch(clearAllChatRuntime());
  store.dispatch(setStatusForUser({ userId: '__pending__', status: 'disconnected' }));
}

describe('ChatRuntimeProvider — artifact event dispatch (#2779)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetRuntimeState();
    vi.mocked(threadApi.getThreads).mockResolvedValue({ threads: [], count: 0 });
  });

  it('onArtifactReady upserts a ready snapshot keyed on the artifact id', () => {
    const listeners = renderProvider();

    act(() => {
      listeners.onArtifactReady?.({
        thread_id: 'thread-1',
        artifact_id: 'a-1',
        kind: 'presentation',
        title: 'Deck',
        workspace_dir: '/workspace',
        path: 'a-1/deck.pptx',
        size_bytes: 4096,
      });
    });

    const bucket = store.getState().chatRuntime.artifactsByThread['thread-1'];
    expect(bucket).toHaveLength(1);
    expect(bucket?.[0]).toMatchObject({
      artifactId: 'a-1',
      kind: 'presentation',
      title: 'Deck',
      path: 'a-1/deck.pptx',
      sizeBytes: 4096,
      status: 'ready',
    });
  });

  it('onArtifactPending upserts an in_progress snapshot keyed on the artifact id (#3162)', () => {
    const listeners = renderProvider();

    act(() => {
      listeners.onArtifactPending?.({
        thread_id: 'thread-1',
        artifact_id: 'a-1',
        kind: 'presentation',
        title: 'Deck',
        workspace_dir: '/workspace',
        path: 'a-1/deck.pptx',
      });
    });

    const bucket = store.getState().chatRuntime.artifactsByThread['thread-1'];
    expect(bucket).toHaveLength(1);
    expect(bucket?.[0]).toMatchObject({
      artifactId: 'a-1',
      kind: 'presentation',
      title: 'Deck',
      status: 'in_progress',
    });
  });

  it('onArtifactFailed records the producer-supplied error', () => {
    const listeners = renderProvider();

    act(() => {
      listeners.onArtifactFailed?.({
        thread_id: 'thread-2',
        artifact_id: 'a-2',
        kind: 'document',
        title: 'Notes',
        workspace_dir: '/workspace',
        error: 'producer crashed',
      });
    });

    const bucket = store.getState().chatRuntime.artifactsByThread['thread-2'];
    expect(bucket).toHaveLength(1);
    expect(bucket?.[0]).toMatchObject({
      artifactId: 'a-2',
      kind: 'document',
      title: 'Notes',
      status: 'failed',
      error: 'producer crashed',
    });
  });

  it('onArtifactFailed dispatches the FULL error to the store even when huge', () => {
    // The provider's rtLog truncates to 80 chars for telemetry, but the
    // dispatched payload must keep the full reason so the ArtifactCard
    // can offer "Show more".
    const listeners = renderProvider();
    const huge = 'p'.repeat(500);

    act(() => {
      listeners.onArtifactFailed?.({
        thread_id: 'thread-3',
        artifact_id: 'a-3',
        kind: 'presentation',
        title: 'Deck',
        workspace_dir: '/workspace',
        error: huge,
      });
    });

    const entry = store.getState().chatRuntime.artifactsByThread['thread-3']?.[0];
    expect(entry?.error).toHaveLength(500);
    expect(entry?.error).toBe(huge);
  });

  it('an artifact_failed → artifact_ready sequence promotes the snapshot in place', () => {
    const listeners = renderProvider();

    act(() => {
      listeners.onArtifactFailed?.({
        thread_id: 'thread-4',
        artifact_id: 'a-4',
        kind: 'presentation',
        title: 'Deck',
        workspace_dir: '/workspace',
        error: 'boom',
      });
    });
    act(() => {
      listeners.onArtifactReady?.({
        thread_id: 'thread-4',
        artifact_id: 'a-4',
        kind: 'presentation',
        title: 'Deck',
        workspace_dir: '/workspace',
        path: 'a-4/deck.pptx',
        size_bytes: 1024,
      });
    });

    const bucket = store.getState().chatRuntime.artifactsByThread['thread-4'];
    expect(bucket).toHaveLength(1);
    expect(bucket?.[0].status).toBe('ready');
    expect(bucket?.[0].error).toBeUndefined();
  });

  it('keeps artifact buckets per-thread', () => {
    const listeners = renderProvider();

    act(() => {
      listeners.onArtifactReady?.({
        thread_id: 'thread-a',
        artifact_id: 'a-1',
        kind: 'image',
        title: 'Pic',
        workspace_dir: '/workspace',
        path: 'a-1/pic.png',
        size_bytes: 1,
      });
      listeners.onArtifactFailed?.({
        thread_id: 'thread-b',
        artifact_id: 'a-2',
        kind: 'document',
        title: 'Doc',
        workspace_dir: '/workspace',
        error: 'denied',
      });
    });

    const state = store.getState().chatRuntime.artifactsByThread;
    expect(state['thread-a']?.[0].status).toBe('ready');
    expect(state['thread-b']?.[0].status).toBe('failed');
    expect(state['thread-a']?.[0].artifactId).toBe('a-1');
    expect(state['thread-b']?.[0].artifactId).toBe('a-2');
  });
});
