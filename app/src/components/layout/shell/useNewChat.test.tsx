import { renderHook, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { AGENT_ACCOUNT_ID } from '../../../utils/accountsFullscreen';
import { useNewChat } from './useNewChat';

// Drive the real hook body while stubbing its router + store dependencies so
// each branch (reuse-empty-thread, create-new-thread) runs deterministically
// without a live core RPC.
const mockNavigate = vi.fn();
vi.mock('react-router-dom', async importOriginal => {
  const actual = await importOriginal<typeof import('react-router-dom')>();
  return { ...actual, useNavigate: () => mockNavigate };
});

interface MockThread {
  id: string;
  messageCount: number;
  labels?: string[];
  parentThreadId?: string;
}

interface MockAction {
  type?: string;
}

interface WrapperProps {
  children: ReactNode;
}

const mockDispatch = vi.fn();
let mockThreads: MockThread[] = [];
let mockMessagesByThreadId: Record<string, unknown[]> = {};
let mockStreamingByThread: Record<string, unknown> = {};
let mockPendingSendThreadIds: Record<string, true> = {};
vi.mock('../../../store/hooks', () => ({
  useAppDispatch: () => mockDispatch,
  useAppSelector: (sel: (s: unknown) => unknown) =>
    sel({
      thread: { threads: mockThreads, messagesByThreadId: mockMessagesByThreadId },
      chatRuntime: {
        streamingAssistantByThread: mockStreamingByThread,
        pendingSendThreadIds: mockPendingSendThreadIds,
      },
    }),
}));

vi.mock('../../../store/accountsSlice', () => ({
  setActiveAccount: vi.fn((id: string) => ({ type: 'accounts/setActiveAccount', payload: id })),
}));
vi.mock('../../../store/threadSlice', () => ({
  createNewThread: vi.fn(() => ({ type: 'thread/createNewThread' })),
  loadThreadMessages: vi.fn((id: string) => ({ type: 'thread/loadThreadMessages', payload: id })),
  setSelectedThread: vi.fn((id: string) => ({ type: 'thread/setSelectedThread', payload: id })),
}));

function wrapper({ children }: WrapperProps) {
  return <MemoryRouter>{children}</MemoryRouter>;
}

function dispatchedTypes() {
  return mockDispatch.mock.calls.map(([action]) => action?.type);
}

describe('useNewChat', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockThreads = [];
    mockMessagesByThreadId = {};
    mockStreamingByThread = {};
    mockPendingSendThreadIds = {};
    mockDispatch.mockImplementation((action: MockAction) => {
      if (action?.type === 'thread/createNewThread') {
        return { unwrap: () => Promise.resolve({ id: 'fresh-thread' }) };
      }
      return undefined;
    });
  });

  it('always switches to the agent account (never reopens a connected-app webview)', () => {
    const { result } = renderHook(() => useNewChat(), { wrapper });
    result.current();
    expect(mockDispatch).toHaveBeenCalledWith({
      type: 'accounts/setActiveAccount',
      payload: AGENT_ACCOUNT_ID,
    });
  });

  it('reuses an existing empty thread regardless of route (does not just /chat)', () => {
    mockThreads = [{ id: 'empty-1', messageCount: 0 }];
    const { result } = renderHook(() => useNewChat(), { wrapper });
    result.current();

    // The key regression: it selects a blank thread and navigates straight to it
    // instead of navigating to bare '/chat' (which would restore the persisted
    // conversation).
    expect(mockNavigate).toHaveBeenCalledWith('/chat/empty-1');
    expect(mockNavigate).not.toHaveBeenCalledWith('/chat');
    expect(mockDispatch).toHaveBeenCalledWith({
      type: 'thread/setSelectedThread',
      payload: 'empty-1',
    });
    expect(dispatchedTypes()).not.toContain('thread/createNewThread');
  });

  it('does not reuse a count-empty thread that already has cached messages', async () => {
    // First message was just sent: messageCount is still a stale 0 but the
    // message cache is populated. Must NOT reuse/reopen it — create instead.
    mockThreads = [{ id: 'just-sent', messageCount: 0 }];
    mockMessagesByThreadId = { 'just-sent': [{ id: 'm1' }] };
    const { result } = renderHook(() => useNewChat(), { wrapper });
    result.current();

    expect(mockNavigate).not.toHaveBeenCalledWith('/chat/just-sent');
    expect(dispatchedTypes()).toContain('thread/createNewThread');
    await waitFor(() => {
      expect(mockNavigate).toHaveBeenCalledWith('/chat/fresh-thread');
    });
  });

  it('does not reuse a count-empty thread that has an in-flight streaming turn', async () => {
    // The thread looks empty (no count, no cache) but a send is in flight
    // (streamingAssistantByThread populated) — never reopen it.
    mockThreads = [{ id: 'sending', messageCount: 0 }];
    mockMessagesByThreadId = {};
    mockStreamingByThread = { sending: { requestId: 'r1', lifecycle: 'started' } };
    const { result } = renderHook(() => useNewChat(), { wrapper });
    result.current();

    expect(mockNavigate).not.toHaveBeenCalledWith('/chat/sending');
    expect(dispatchedTypes()).toContain('thread/createNewThread');
    await waitFor(() => {
      expect(mockNavigate).toHaveBeenCalledWith('/chat/fresh-thread');
    });
  });

  it('does not reuse a thread with an optimistic send pending (pre-fulfillment window)', async () => {
    // Earliest window: send recorded in pendingSendThreadIds before
    // addMessageLocal resolves and before any streaming state exists.
    mockThreads = [{ id: 'optimistic', messageCount: 0 }];
    mockMessagesByThreadId = {};
    mockStreamingByThread = {};
    mockPendingSendThreadIds = { optimistic: true };
    const { result } = renderHook(() => useNewChat(), { wrapper });
    result.current();

    expect(mockNavigate).not.toHaveBeenCalledWith('/chat/optimistic');
    expect(dispatchedTypes()).toContain('thread/createNewThread');
    await waitFor(() => {
      expect(mockNavigate).toHaveBeenCalledWith('/chat/fresh-thread');
    });
  });

  it('does not reuse a blank non-General thread (task / subconscious / parented)', async () => {
    // A blank task thread (parentThreadId) is hidden from the General tab, so
    // New Chat must not land on it — create a fresh general chat instead.
    mockThreads = [{ id: 'task-1', messageCount: 0, parentThreadId: 'parent' }];
    const { result } = renderHook(() => useNewChat(), { wrapper });
    result.current();

    expect(mockNavigate).not.toHaveBeenCalledWith('/chat/task-1');
    expect(dispatchedTypes()).toContain('thread/createNewThread');
    await waitFor(() => {
      expect(mockNavigate).toHaveBeenCalledWith('/chat/fresh-thread');
    });
  });

  it('reuses a genuinely-blank thread (count-empty, no cache, no in-flight turn)', () => {
    // Round-trip of the earlier review: a blank current chat must be reused
    // rather than spawning yet another empty thread.
    mockThreads = [{ id: 'blank', messageCount: 0 }];
    mockMessagesByThreadId = {};
    mockStreamingByThread = {};
    const { result } = renderHook(() => useNewChat(), { wrapper });
    result.current();

    expect(mockNavigate).toHaveBeenCalledWith('/chat/blank');
    expect(dispatchedTypes()).not.toContain('thread/createNewThread');
  });

  it('creates a new thread when there is no empty thread', async () => {
    mockThreads = [{ id: 't-busy', messageCount: 4 }];
    const { result } = renderHook(() => useNewChat(), { wrapper });
    result.current();

    expect(dispatchedTypes()).toContain('thread/createNewThread');
    await waitFor(() => {
      expect(mockDispatch).toHaveBeenCalledWith({
        type: 'thread/setSelectedThread',
        payload: 'fresh-thread',
      });
    });
    expect(mockDispatch).toHaveBeenCalledWith({
      type: 'thread/loadThreadMessages',
      payload: 'fresh-thread',
    });
    expect(mockNavigate).toHaveBeenCalledWith('/chat/fresh-thread');
  });

  it('swallows a failed thread creation without throwing', async () => {
    mockThreads = [{ id: 't-busy', messageCount: 2 }];
    mockDispatch.mockImplementation((action: MockAction) => {
      if (action?.type === 'thread/createNewThread') {
        return { unwrap: () => Promise.reject(new Error('boom')) };
      }
      return undefined;
    });
    const { result } = renderHook(() => useNewChat(), { wrapper });
    expect(() => result.current()).not.toThrow();
    await Promise.resolve();
    expect(dispatchedTypes()).toContain('thread/createNewThread');
  });
});
