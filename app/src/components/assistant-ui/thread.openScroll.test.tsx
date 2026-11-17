/** Native assistant-ui initialization and cached-thread switching over Redux. */
import { AssistantUiRuntimeProvider } from '@/providers/AssistantUiRuntimeProvider';
import chatRuntimeReducer from '@/store/chatRuntimeSlice';
import threadReducer, { loadThreadMessages } from '@/store/threadSlice';
import type { ThreadMessage } from '@/types/thread';
import { configureStore } from '@reduxjs/toolkit';
import { render, waitFor } from '@testing-library/react';
import { act } from 'react';
import { Provider } from 'react-redux';
import { afterEach, beforeEach, describe, expect, it, type MockInstance, vi } from 'vitest';

import { Thread } from './thread';

vi.mock('@/services/api/threadApi', () => ({
  threadApi: { getDerivedTranscript: vi.fn().mockResolvedValue({ items: [], nextCursor: null }) },
}));

function msg(id: string, sender: 'user' | 'agent', content: string): ThreadMessage {
  return {
    id,
    content,
    type: 'text',
    extraMetadata: {},
    sender,
    createdAt: `2026-09-23T00:00:${id.padStart(2, '0')}Z`,
  };
}

/** Seed a thread's messages exactly as a real fetch would, without the fetch. */
function cacheThread(
  store: ReturnType<typeof makeStore>,
  threadId: string,
  messages: ThreadMessage[]
) {
  act(() => {
    store.dispatch({ type: loadThreadMessages.fulfilled.type, payload: { threadId, messages } });
  });
}

function makeStore() {
  return configureStore({ reducer: { thread: threadReducer, chatRuntime: chatRuntimeReducer } });
}

/**
 * The viewport is created by `<Thread />` itself, so the spy has to be on the
 * prototype. Height metrics are zero in jsdom; tests that care about the
 * reader's position set them per-element.
 */
let scrollToSpy: MockInstance;
let scrollIntoViewSpy: MockInstance;

beforeEach(() => {
  scrollToSpy = vi.spyOn(HTMLElement.prototype, 'scrollTo').mockImplementation(() => {});
  scrollIntoViewSpy = vi.spyOn(Element.prototype, 'scrollIntoView').mockImplementation(() => {});
});

afterEach(() => {
  scrollToSpy.mockRestore();
  scrollIntoViewSpy.mockRestore();
});

function viewportOf(container: HTMLElement): HTMLElement {
  const el = container.querySelector<HTMLElement>('[data-slot="aui_thread-viewport"]');
  if (!el) throw new Error('viewport not rendered');
  return el;
}

/** Did anything ask this viewport to jump to its own bottom? */
function scrolledToBottom(viewport: HTMLElement): boolean {
  // `mock.contexts`, not `mock.instances`: the latter only records calls made
  // with `new`, so it is empty for a method spy and would make this vacuously
  // false for every viewport.
  return scrollToSpy.mock.calls.some((call, i) => {
    if (scrollToSpy.mock.contexts[i] !== viewport) return false;
    const options = call[0] as ScrollToOptions | undefined;
    return typeof options === 'object' && options !== null && options.top === viewport.scrollHeight;
  });
}

describe('opening a thread scrolls to the newest message', () => {
  it('scrolls to the bottom when the last message is from the assistant', async () => {
    const store = makeStore();
    cacheThread(store, 't-assistant', [
      msg('1', 'user', 'question'),
      msg('2', 'agent', 'a long answer'),
    ]);

    const { container } = render(
      <Provider store={store}>
        <AssistantUiRuntimeProvider threadId="t-assistant">
          <Thread />
        </AssistantUiRuntimeProvider>
      </Provider>
    );

    await waitFor(() => expect(scrolledToBottom(viewportOf(container))).toBe(true));
  });

  it('scrolls when switching to an ALREADY-CACHED thread, which never empties', async () => {
    const store = makeStore();
    cacheThread(store, 't-a', [msg('1', 'user', 'first thread')]);
    cacheThread(store, 't-b', [msg('2', 'user', 'hi'), msg('3', 'agent', 'second thread')]);

    const { container, rerender } = render(
      <Provider store={store}>
        <AssistantUiRuntimeProvider threadId="t-a">
          <Thread />
        </AssistantUiRuntimeProvider>
      </Provider>
    );

    const viewport = viewportOf(container);
    await waitFor(() => expect(scrolledToBottom(viewport)).toBe(true));
    // The opening scroll for t-a is not what this test is about.
    scrollToSpy.mockClear();

    rerender(
      <Provider store={store}>
        <AssistantUiRuntimeProvider threadId="t-b">
          <Thread />
        </AssistantUiRuntimeProvider>
      </Provider>
    );

    // Same viewport element across the switch — no remount laundered the state.
    expect(viewportOf(container)).toBe(viewport);
    // Both threads are cached, so t-b's messages were present on the render the
    // id flipped: the adapter supplies it to the native thread-switch event.
    await waitFor(() => expect(scrolledToBottom(viewport)).toBe(true));
  });

  it('does not yank a reader who has scrolled up when a new turn arrives', async () => {
    const store = makeStore();
    cacheThread(store, 't-reader', [msg('1', 'user', 'q'), msg('2', 'agent', 'a')]);

    const { container } = render(
      <Provider store={store}>
        <AssistantUiRuntimeProvider threadId="t-reader">
          <Thread />
        </AssistantUiRuntimeProvider>
      </Provider>
    );

    // Put the reader far up the transcript: 1000px of content, 200px tall
    // viewport, parked at the top — 800px from the bottom.
    const viewport = viewportOf(container);
    await waitFor(() => expect(scrolledToBottom(viewport)).toBe(true));
    Object.defineProperty(viewport, 'scrollHeight', { value: 1000, configurable: true });
    Object.defineProperty(viewport, 'clientHeight', { value: 200, configurable: true });
    viewport.scrollTop = 800;
    act(() => viewport.dispatchEvent(new Event('scroll')));
    viewport.scrollTop = 0;
    act(() => viewport.dispatchEvent(new Event('scroll')));
    scrollToSpy.mockClear();
    scrollIntoViewSpy.mockClear();

    // A new user turn lands in the SAME thread — no thread switch.
    cacheThread(store, 't-reader', [
      msg('1', 'user', 'q'),
      msg('2', 'agent', 'a'),
      msg('4', 'user', 'follow-up'),
    ]);

    expect(scrollIntoViewSpy).not.toHaveBeenCalled();
    expect(scrolledToBottom(viewport)).toBe(false);
  });

  it('follows a new turn at the bottom without a custom top alignment', async () => {
    const store = makeStore();
    cacheThread(store, 't-bottom', [msg('1', 'user', 'q'), msg('2', 'agent', 'a')]);

    const { container } = render(
      <Provider store={store}>
        <AssistantUiRuntimeProvider threadId="t-bottom">
          <Thread />
        </AssistantUiRuntimeProvider>
      </Provider>
    );

    const viewport = viewportOf(container);
    await waitFor(() => expect(scrolledToBottom(viewport)).toBe(true));
    Object.defineProperty(viewport, 'scrollHeight', { value: 1000, configurable: true });
    Object.defineProperty(viewport, 'clientHeight', { value: 200, configurable: true });
    viewport.scrollTop = 800; // pinned to the bottom
    act(() => viewport.dispatchEvent(new Event('scroll')));
    scrollToSpy.mockClear();
    scrollIntoViewSpy.mockClear();

    cacheThread(store, 't-bottom', [
      msg('1', 'user', 'q'),
      msg('2', 'agent', 'a'),
      msg('4', 'user', 'follow-up'),
    ]);

    Object.defineProperty(viewport, 'scrollHeight', { value: 1200, configurable: true });
    await waitFor(() => expect(scrolledToBottom(viewport)).toBe(true));
    expect(scrollIntoViewSpy).not.toHaveBeenCalled();
  });
});
