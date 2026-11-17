/** Behaviour tests for assistant-ui's native viewport over the OpenHuman adapter.
 * jsdom has no layout, so scroll geometry and resize notifications are supplied.
 */
import { AssistantUiRuntimeProvider } from '@/providers/AssistantUiRuntimeProvider';
import chatRuntimeReducer from '@/store/chatRuntimeSlice';
import threadReducer, { loadThreadMessages } from '@/store/threadSlice';
import type { ThreadMessage } from '@/types/thread';
import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen } from '@testing-library/react';
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

function makeStore() {
  return configureStore({ reducer: { thread: threadReducer, chatRuntime: chatRuntimeReducer } });
}

/**
 * Capture every `ResizeObserver` callback the tree installs, so content growth
 * can be simulated. The shared setup polyfills a no-op observer, which would
 * make these tests vacuously green — nothing would ever fire.
 */
let resizeCallbacks: ResizeObserverCallback[] = [];
let scrollToSpy: MockInstance;
const RealResizeObserver = globalThis.ResizeObserver;

beforeEach(() => {
  resizeCallbacks = [];
  globalThis.ResizeObserver = class {
    constructor(cb: ResizeObserverCallback) {
      resizeCallbacks.push(cb);
    }
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
  scrollToSpy = vi.spyOn(HTMLElement.prototype, 'scrollTo').mockImplementation(() => {});
  vi.spyOn(Element.prototype, 'scrollIntoView').mockImplementation(() => {});
});

afterEach(() => {
  globalThis.ResizeObserver = RealResizeObserver;
  vi.restoreAllMocks();
});

function viewportOf(container: HTMLElement): HTMLElement {
  const el = container.querySelector<HTMLElement>('[data-slot="aui_thread-viewport"]');
  if (!el) throw new Error('viewport not rendered');
  return el;
}

/** jsdom reports 0 for every box, so the reader's position is set outright. */
function setGeometry(el: HTMLElement, scrollTop: number, scrollHeight: number, clientHeight = 500) {
  Object.defineProperty(el, 'scrollHeight', { value: scrollHeight, configurable: true });
  Object.defineProperty(el, 'clientHeight', { value: clientHeight, configurable: true });
  el.scrollTop = scrollTop;
}

/**
 * The reader scrolling: an input that can scroll (a wheel notch here), then the
 * `scroll` event it produced. A bare `scroll` event is a layout shift or
 * someone else's programmatic scroll, not the reader.
 */
function readerScrolls(viewport: HTMLElement) {
  act(() => {
    viewport.dispatchEvent(new WheelEvent('wheel', { deltaY: -100 }));
    viewport.dispatchEvent(new Event('scroll'));
  });
}

/** Fire every captured resize callback, as a height change would. */
function growContent() {
  act(() => {
    for (const cb of resizeCallbacks) cb([], {} as ResizeObserver);
  });
}

function followedBottom(viewport: HTMLElement): boolean {
  return scrollToSpy.mock.calls.some((call, i) => {
    if (scrollToSpy.mock.contexts[i] !== viewport) return false;
    const options = call[0] as ScrollToOptions | undefined;
    return options?.top === viewport.scrollHeight;
  });
}

function renderThread() {
  const store = makeStore();
  act(() => {
    store.dispatch({
      type: loadThreadMessages.fulfilled.type,
      payload: { threadId: 't1', messages: [msg('1', 'user', 'hi'), msg('2', 'agent', 'hello')] },
    });
  });
  const { container } = render(
    <Provider store={store}>
      <AssistantUiRuntimeProvider threadId="t1">
        <Thread />
      </AssistantUiRuntimeProvider>
    </Provider>
  );
  return { container, store, viewport: viewportOf(container) };
}

describe('assistant-ui native scroll anchoring', () => {
  it('offers a labelled scroll anchor away from the bottom and jumps instantly', () => {
    const { viewport } = renderThread();
    setGeometry(viewport, 500, 1000);
    act(() => viewport.dispatchEvent(new Event('scroll')));
    expect(screen.getByRole('button', { name: 'Scroll to bottom' })).toBeDisabled();

    setGeometry(viewport, 100, 1000);
    readerScrolls(viewport);
    const anchor = screen.getByRole('button', { name: 'Scroll to bottom' });
    expect(anchor).toBeEnabled();
    expect(anchor).toHaveTextContent('Scroll to bottom');
    scrollToSpy.mockClear();
    fireEvent.click(anchor);
    expect(scrollToSpy).toHaveBeenCalledWith({ top: 1000, behavior: 'instant' });
  });

  it('resumes following after clicking the scroll anchor', () => {
    const { viewport } = renderThread();
    setGeometry(viewport, 500, 1000);
    act(() => viewport.dispatchEvent(new Event('scroll')));
    setGeometry(viewport, 100, 1000);
    readerScrolls(viewport);
    scrollToSpy.mockImplementation(function (this: HTMLElement, options: ScrollToOptions) {
      this.scrollTop = Math.min(options.top ?? 0, this.scrollHeight - this.clientHeight);
    });
    fireEvent.click(screen.getByRole('button', { name: 'Scroll to bottom' }));
    act(() => viewport.dispatchEvent(new Event('scroll')));
    expect(screen.getByRole('button', { name: 'Scroll to bottom' })).toBeDisabled();
    scrollToSpy.mockClear();
    setGeometry(viewport, 500, 1400);
    growContent();
    expect(followedBottom(viewport)).toBe(true);
  });

  it('follows content growth for a reader at the bottom', () => {
    const { viewport } = renderThread();
    setGeometry(viewport, 500, 1000);
    act(() => viewport.dispatchEvent(new Event('scroll')));
    scrollToSpy.mockClear();

    setGeometry(viewport, 500, 1400);
    growContent();

    expect(followedBottom(viewport)).toBe(true);
  });

  it('stops following once the reader scrolls up', () => {
    const { viewport } = renderThread();
    setGeometry(viewport, 500, 1000);
    act(() => viewport.dispatchEvent(new Event('scroll')));

    // A deliberate move into history.
    setGeometry(viewport, 100, 1000);
    readerScrolls(viewport);
    scrollToSpy.mockClear();

    setGeometry(viewport, 100, 1400);
    growContent();

    expect(followedBottom(viewport)).toBe(false);
  });

  it('resumes following when the reader returns to the bottom', () => {
    const { viewport } = renderThread();
    // Start AT the bottom and scroll away, so following is genuinely off before
    // the return is tested. Going straight to 100 from the initial baseline of
    // 0 is an INCREASE, which never trips the clearing branch — the test would
    // then pass with the proximity branch deleted.
    setGeometry(viewport, 500, 1000);
    act(() => viewport.dispatchEvent(new Event('scroll')));

    setGeometry(viewport, 100, 1000);
    readerScrolls(viewport);

    setGeometry(viewport, 500, 1000);
    readerScrolls(viewport);
    scrollToSpy.mockClear();

    setGeometry(viewport, 500, 1400);
    growContent();

    expect(followedBottom(viewport)).toBe(true);
  });

  it('follows new user-message growth at the bottom without a custom top alignment', () => {
    const { store, viewport } = renderThread();
    setGeometry(viewport, 500, 1000);
    act(() => viewport.dispatchEvent(new Event('scroll')));
    scrollToSpy.mockClear();
    const alignment = vi.spyOn(Element.prototype, 'scrollIntoView');
    setGeometry(viewport, 500, 1400);
    act(() =>
      store.dispatch({
        type: loadThreadMessages.fulfilled.type,
        payload: {
          threadId: 't1',
          messages: [
            msg('1', 'user', 'hi'),
            msg('2', 'agent', 'hello'),
            msg('3', 'user', 'follow-up'),
          ],
        },
      })
    );
    growContent();
    expect(followedBottom(viewport)).toBe(true);
    expect(alignment).not.toHaveBeenCalled();
  });

  it('keeps following when collapsing content changes the scroll geometry', () => {
    const { viewport } = renderThread();
    setGeometry(viewport, 500, 1000);
    act(() => viewport.dispatchEvent(new Event('scroll')));

    setGeometry(viewport, 100, 600);
    act(() => viewport.dispatchEvent(new Event('scroll')));
    scrollToSpy.mockClear();

    setGeometry(viewport, 100, 1400);
    growContent();

    expect(followedBottom(viewport)).toBe(true);
  });

  it('stops following on a keyboard scroll up', () => {
    const { viewport } = renderThread();
    setGeometry(viewport, 500, 1000);
    act(() => viewport.dispatchEvent(new Event('scroll')));

    setGeometry(viewport, 100, 1000);
    act(() => {
      viewport.ownerDocument.dispatchEvent(new KeyboardEvent('keydown', { key: 'PageUp' }));
      viewport.dispatchEvent(new Event('scroll'));
    });
    scrollToSpy.mockClear();

    setGeometry(viewport, 100, 1400);
    growContent();

    expect(followedBottom(viewport)).toBe(false);
  });

  it('does not treat a growth-only scroll event as the reader leaving', () => {
    const { viewport } = renderThread();
    setGeometry(viewport, 500, 1000);
    act(() => viewport.dispatchEvent(new Event('scroll')));

    // The reply grew by more than the threshold and a queued `scroll` runs
    // before the resize callback. `scrollTop` has NOT moved, so this must not
    // clear the flag even though the distance now reads far from the bottom.
    setGeometry(viewport, 500, 1400);
    act(() => viewport.dispatchEvent(new Event('scroll')));
    scrollToSpy.mockClear();

    growContent();

    expect(followedBottom(viewport)).toBe(true);
  });
});
