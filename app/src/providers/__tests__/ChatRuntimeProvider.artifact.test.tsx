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

// Mirrors the harness in ChatRuntimeProvider.test.tsx but scoped to the
// artifact handlers (#3024 workspace_dir binding). We need the listeners
// captured by the real `subscribeChatEvents` call to fire `onArtifactReady`
// / `onArtifactFailed` end-to-end through the provider — otherwise the
// handler bodies (797-825) stay at 0% diff-cover.

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

describe('ChatRuntimeProvider — artifact lifecycle (#3024)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetRuntimeState();
    vi.mocked(threadApi.getThreads).mockResolvedValue({ threads: [], count: 0 });
  });

  it('onArtifactReady upserts a ready snapshot into the thread bucket', () => {
    const listeners = renderProvider();

    act(() => {
      listeners.onArtifactReady?.({
        thread_id: 'thread-artifact',
        client_id: 'socket-1',
        artifact_id: 'art-1',
        kind: 'presentation',
        title: 'Climate Deck',
        workspace_dir: '/workspace',
        path: 'artifacts/art-1.pptx',
        size_bytes: 4096,
      });
    });

    const bucket = store.getState().chatRuntime.artifactsByThread['thread-artifact'] ?? [];
    expect(bucket).toHaveLength(1);
    expect(bucket[0]).toMatchObject({
      artifactId: 'art-1',
      kind: 'presentation',
      title: 'Climate Deck',
      status: 'ready',
      path: 'artifacts/art-1.pptx',
      sizeBytes: 4096,
    });
  });

  it('onArtifactFailed upserts a failed snapshot with the producer error', () => {
    const listeners = renderProvider();

    act(() => {
      listeners.onArtifactFailed?.({
        thread_id: 'thread-fail',
        client_id: 'socket-1',
        artifact_id: 'art-2',
        kind: 'document',
        title: 'Quarterly Report',
        workspace_dir: '/workspace',
        error: 'pip install crashed',
      });
    });

    const bucket = store.getState().chatRuntime.artifactsByThread['thread-fail'] ?? [];
    expect(bucket).toHaveLength(1);
    expect(bucket[0]).toMatchObject({
      artifactId: 'art-2',
      kind: 'document',
      title: 'Quarterly Report',
      status: 'failed',
      error: 'pip install crashed',
    });
  });

  // The defence-in-depth slice(0, 80) in the provider's onArtifactFailed
  // body protects the dispatch logging, not the redux payload — assert
  // the full producer error still lands in the store for the UI to render.
  it('onArtifactFailed preserves the FULL producer error in redux (logging is capped, not the payload)', () => {
    const listeners = renderProvider();
    const longError = 'x'.repeat(500);

    act(() => {
      listeners.onArtifactFailed?.({
        thread_id: 'thread-long',
        client_id: 'socket-1',
        artifact_id: 'art-3',
        kind: 'image',
        title: 'Render',
        workspace_dir: '/workspace',
        error: longError,
      });
    });

    const bucket = store.getState().chatRuntime.artifactsByThread['thread-long'] ?? [];
    expect(bucket).toHaveLength(1);
    expect(bucket[0]?.error).toBe(longError);
  });
});
