/**
 * The write half of the assistant-ui seam.
 *
 * `useOpenHumanExternalStore`'s `onNew` throws when no surface owns the thread,
 * so these tests are what make the runtime's action API real: they pin that the
 * chat surface claims its thread, releases it, hands it over on a thread switch,
 * and that a turn appended through the runtime lands on the surface's own send
 * function rather than on a copy of it.
 */
import { useAui } from '@assistant-ui/react';
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, render, renderHook, screen } from '@testing-library/react';
import { useRef } from 'react';
import { Provider } from 'react-redux';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { AssistantUiRuntimeProvider } from '../../../providers/AssistantUiRuntimeProvider';
import { __resetChatSurfaces, getChatSurface } from '../../../providers/chatSurfaceHandlers';
import chatRuntimeReducer from '../../../store/chatRuntimeSlice';
import threadReducer from '../../../store/threadSlice';
import { useChatSurfaceRegistration } from './useChatSurfaceRegistration';

afterEach(() => __resetChatSurfaces());

/** Drives the hook with refs the test can point at spies. */
function useHarness(
  threadId: string | null,
  send: (text?: string) => Promise<void>,
  stop: () => void,
  registerWithoutThread = false
) {
  const sendRef = useRef<((text?: string) => Promise<void>) | null>(null);
  const stopRef = useRef<(() => void) | null>(null);
  // Assigned during render, exactly as `Conversations.tsx` does it.
  sendRef.current = send;
  stopRef.current = stop;
  useChatSurfaceRegistration(threadId, sendRef, stopRef, registerWithoutThread);
}

describe('useChatSurfaceRegistration', () => {
  it('registers the surface for the active thread', () => {
    const send = vi.fn(async () => {});
    renderHook(() => useHarness('t-1', send, () => {}));

    expect(getChatSurface('t-1')).not.toBeNull();
  });

  it('unregisters on unmount', () => {
    const { unmount } = renderHook(() =>
      useHarness(
        't-1',
        async () => {},
        () => {}
      )
    );
    expect(getChatSurface('t-1')).not.toBeNull();

    unmount();

    expect(getChatSurface('t-1')).toBeNull();
  });

  it('moves the registration when the thread changes', () => {
    const { rerender } = renderHook(
      ({ id }: { id: string }) =>
        useHarness(
          id,
          async () => {},
          () => {}
        ),
      { initialProps: { id: 't-1' } }
    );
    expect(getChatSurface('t-1')).not.toBeNull();

    rerender({ id: 't-2' });

    expect(getChatSurface('t-1')).toBeNull();
    expect(getChatSurface('t-2')).not.toBeNull();
  });

  it('registers nothing when no thread is selected', () => {
    renderHook(() =>
      useHarness(
        null,
        async () => {},
        () => {}
      )
    );

    expect(getChatSurface(null)).toBeNull();
  });

  it('can register the home composer before its first thread exists', async () => {
    const send = vi.fn(async () => {});
    renderHook(() => useHarness(null, send, () => {}, true));

    await getChatSurface(null)?.send('/new');

    expect(send).toHaveBeenCalledWith('/new');
  });

  it('keeps the registered identity stable across re-renders with fresh closures', () => {
    const { rerender } = renderHook(
      ({ send }: { send: () => Promise<void> }) => useHarness('t-1', send, () => {}),
      { initialProps: { send: async () => {} } }
    );
    const first = getChatSurface('t-1');

    // A keystroke re-render hands the hook brand-new closures. The registry
    // entry must NOT be rewritten — churn there can drop the slot mid-turn.
    rerender({ send: async () => {} });
    rerender({ send: async () => {} });

    expect(getChatSurface('t-1')).toBe(first);
  });

  it('forwards send to the latest composer send function', async () => {
    const stale = vi.fn(async () => {});
    const fresh = vi.fn(async () => {});
    const { rerender } = renderHook(
      ({ send }: { send: () => Promise<void> }) => useHarness('t-1', send, () => {}),
      { initialProps: { send: stale } }
    );
    rerender({ send: fresh });

    await act(async () => {
      await getChatSurface('t-1')?.send('hello');
    });

    expect(fresh).toHaveBeenCalledWith('hello');
    expect(stale).not.toHaveBeenCalled();
  });

  it('forwards cancel to the surface stop-generation function', async () => {
    const stop = vi.fn();
    renderHook(() => useHarness('t-1', async () => {}, stop));

    await act(async () => {
      await getChatSurface('t-1')?.cancel?.();
    });

    expect(stop).toHaveBeenCalledTimes(1);
  });

  it('exposes no reload — this surface has no regenerate-last-turn path', () => {
    renderHook(() =>
      useHarness(
        't-1',
        async () => {},
        () => {}
      )
    );

    expect(getChatSurface('t-1')?.reload).toBeUndefined();
  });
});

const RUNTIME_THREAD = 't-runtime';

function buildStore() {
  return configureStore({
    reducer: combineReducers({ thread: threadReducer, chatRuntime: chatRuntimeReducer }),
    preloadedState: {
      thread: {
        threads: [],
        selectedThreadId: RUNTIME_THREAD,
        activeThreadIds: {},
        welcomeThreadId: null,
        messagesByThreadId: { [RUNTIME_THREAD]: [] },
        messages: [],
        isLoadingThreads: false,
        isLoadingMessages: false,
        messagesError: null,
      },
    } as never,
  });
}

describe('useChatSurfaceRegistration through the assistant-ui runtime', () => {
  it('routes an appended turn to the surface send path', async () => {
    const send = vi.fn(async () => {});

    function Surface() {
      useHarness(RUNTIME_THREAD, send, () => {});
      const aui = useAui();
      return (
        <button
          type="button"
          data-testid="append"
          onClick={() =>
            void aui.thread.append({ role: 'user', content: [{ type: 'text', text: 'hi there' }] })
          }>
          append
        </button>
      );
    }

    render(
      <Provider store={buildStore()}>
        <AssistantUiRuntimeProvider>
          <Surface />
        </AssistantUiRuntimeProvider>
      </Provider>
    );

    await act(async () => {
      screen.getByTestId('append').click();
    });

    expect(send).toHaveBeenCalledWith('hi there');
  });
});
