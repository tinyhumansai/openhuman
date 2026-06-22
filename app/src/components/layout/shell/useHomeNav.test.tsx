import { renderHook, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { AGENT_ACCOUNT_ID } from '../../../utils/accountsFullscreen';
import { useHomeNav } from './useHomeNav';

// --- Mocks ------------------------------------------------------------------
// Drive the real hook body while stubbing its router + store dependencies so
// each branch (off-chat navigate, reuse-empty-thread, create-new-thread) runs
// deterministically without a live core RPC.
const mockNavigate = vi.fn();
let mockPathname = '/home';
vi.mock('react-router-dom', async importOriginal => {
  const actual = await importOriginal<typeof import('react-router-dom')>();
  return {
    ...actual,
    useNavigate: () => mockNavigate,
    useLocation: () => ({ pathname: mockPathname }),
  };
});

interface MockThread {
  id: string;
  messageCount: number;
}

interface MockAction {
  type?: string;
}

interface WrapperProps {
  children: ReactNode;
}

const mockDispatch = vi.fn();
let mockThreads: MockThread[] = [];
vi.mock('../../../store/hooks', () => ({
  useAppDispatch: () => mockDispatch,
  useAppSelector: (sel: (s: unknown) => unknown) => sel({ thread: { threads: mockThreads } }),
}));

// Tag each action creator so we can assert what was dispatched.
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

describe('useHomeNav', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockPathname = '/home';
    mockThreads = [];
    // createNewThread dispatch yields a thunk-style promise with .unwrap().
    mockDispatch.mockImplementation((action: MockAction) => {
      if (action?.type === 'thread/createNewThread') {
        return { unwrap: () => Promise.resolve({ id: 'fresh-thread' }) };
      }
      return undefined;
    });
  });

  it('switches to the agent account and navigates to chat when off-chat', () => {
    mockPathname = '/home';
    const { result } = renderHook(() => useHomeNav(), { wrapper });
    result.current();

    expect(dispatchedTypes()).toContain('accounts/setActiveAccount');
    expect(mockDispatch).toHaveBeenCalledWith({
      type: 'accounts/setActiveAccount',
      payload: AGENT_ACCOUNT_ID,
    });
    expect(mockNavigate).toHaveBeenCalledWith('/chat');
    // Off-chat path returns before touching threads.
    expect(dispatchedTypes()).not.toContain('thread/createNewThread');
    expect(dispatchedTypes()).not.toContain('thread/setSelectedThread');
  });

  it('reuses an existing empty thread when already on chat', () => {
    mockPathname = '/chat';
    mockThreads = [{ id: 'empty-1', messageCount: 0 }];
    const { result } = renderHook(() => useHomeNav(), { wrapper });
    result.current();

    expect(mockNavigate).toHaveBeenCalledWith('/chat/empty-1');
    expect(mockDispatch).toHaveBeenCalledWith({
      type: 'thread/setSelectedThread',
      payload: 'empty-1',
    });
    expect(mockDispatch).toHaveBeenCalledWith({
      type: 'thread/loadThreadMessages',
      payload: 'empty-1',
    });
    expect(dispatchedTypes()).not.toContain('thread/createNewThread');
  });

  it('creates a new thread when on chat with no empty thread', async () => {
    mockPathname = '/chat/abc';
    mockThreads = [{ id: 't-busy', messageCount: 4 }];
    const { result } = renderHook(() => useHomeNav(), { wrapper });
    result.current();

    expect(dispatchedTypes()).toContain('thread/createNewThread');
    // The freshly created thread is selected + loaded once the promise resolves.
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
    mockPathname = '/chat';
    mockThreads = [{ id: 't-busy', messageCount: 2 }];
    mockDispatch.mockImplementation((action: MockAction) => {
      if (action?.type === 'thread/createNewThread') {
        return { unwrap: () => Promise.reject(new Error('boom')) };
      }
      return undefined;
    });
    const { result } = renderHook(() => useHomeNav(), { wrapper });
    expect(() => result.current()).not.toThrow();
    // Give the rejected promise a tick to settle through the .catch().
    await Promise.resolve();
    expect(dispatchedTypes()).toContain('thread/createNewThread');
  });
});
