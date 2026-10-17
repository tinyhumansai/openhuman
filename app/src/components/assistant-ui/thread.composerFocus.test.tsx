/** The composer takes the caret once per conversation open, and never steals it. */
import { AssistantUiRuntimeProvider } from '@/providers/AssistantUiRuntimeProvider';
import chatRuntimeReducer from '@/store/chatRuntimeSlice';
import threadReducer, { loadThreadMessages } from '@/store/threadSlice';
import type { ThreadMessage } from '@/types/thread';
import {
  AssistantRuntimeProvider,
  type ThreadMessageLike,
  useExternalStoreRuntime,
} from '@assistant-ui/react';
import { configureStore } from '@reduxjs/toolkit';
import { render, screen, waitFor } from '@testing-library/react';
import { act, type ReactNode } from 'react';
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
    createdAt: `2026-10-14T00:00:${id.padStart(2, '0')}Z`,
  };
}

function makeStore() {
  return configureStore({ reducer: { thread: threadReducer, chatRuntime: chatRuntimeReducer } });
}

type Store = ReturnType<typeof makeStore>;

function cacheThread(store: Store, threadId: string, messages: ThreadMessage[]) {
  act(() => {
    store.dispatch({ type: loadThreadMessages.fulfilled.type, payload: { threadId, messages } });
  });
}

/** The thread under test, plus whatever the page around it holds (a modal, a search box). */
function ui(store: Store, threadId: string, around?: ReactNode) {
  return (
    <Provider store={store}>
      {around}
      <AssistantUiRuntimeProvider threadId={threadId}>
        <Thread />
      </AssistantUiRuntimeProvider>
    </Provider>
  );
}

function composer() {
  return screen.getByTestId('chat-message-input');
}

function blurAll() {
  act(() => (document.activeElement as HTMLElement | null)?.blur());
  expect(document.activeElement).toBe(document.body);
}

let scrollToSpy: MockInstance;
let scrollIntoViewSpy: MockInstance;
let focusSpy: MockInstance;

beforeEach(() => {
  scrollToSpy = vi.spyOn(HTMLElement.prototype, 'scrollTo').mockImplementation(() => {});
  scrollIntoViewSpy = vi.spyOn(Element.prototype, 'scrollIntoView').mockImplementation(() => {});
  focusSpy = vi.spyOn(HTMLElement.prototype, 'focus');
});

afterEach(() => {
  scrollToSpy.mockRestore();
  scrollIntoViewSpy.mockRestore();
  focusSpy.mockRestore();
});

describe('composer focus on conversation open', () => {
  it('focuses the composer when a conversation is opened, without scrolling to it', async () => {
    const store = makeStore();
    cacheThread(store, 't-open', [msg('1', 'user', 'q'), msg('2', 'agent', 'a')]);

    render(ui(store, 't-open'));

    await waitFor(() => expect(composer()).toHaveFocus());
    const call = focusSpy.mock.calls.find((_, i) => focusSpy.mock.contexts[i] === composer());
    expect(call?.[0]).toEqual({ preventScroll: true });
  });

  it('focuses the composer again when switching conversations', async () => {
    const store = makeStore();
    cacheThread(store, 't-a', [msg('1', 'user', 'first')]);
    cacheThread(store, 't-b', [msg('2', 'user', 'hi'), msg('3', 'agent', 'second')]);

    const { rerender } = render(ui(store, 't-a'));
    await waitFor(() => expect(composer()).toHaveFocus());
    // Clicking a row in the thread list moves focus off the composer first.
    blurAll();

    rerender(ui(store, 't-b'));

    await waitFor(() => expect(composer()).toHaveFocus());
  });

  it('focuses the composer when New chat lands on an empty conversation', async () => {
    const store = makeStore();
    cacheThread(store, 't-old', [msg('1', 'user', 'old'), msg('2', 'agent', 'reply')]);
    cacheThread(store, 't-new', []);

    const { rerender } = render(ui(store, 't-old'));
    await waitFor(() => expect(composer()).toHaveFocus());
    blurAll();

    rerender(ui(store, 't-new'));

    await waitFor(() => expect(composer()).toHaveFocus());
  });

  it('does not take focus from another input on the page', async () => {
    const store = makeStore();
    cacheThread(store, 't-a', [msg('1', 'user', 'first')]);
    cacheThread(store, 't-b', [msg('2', 'user', 'second')]);
    const search = <input aria-label="Search conversations" />;

    const { rerender } = render(ui(store, 't-a', search));
    await waitFor(() => expect(composer()).toHaveFocus());
    const box = screen.getByLabelText('Search conversations');
    act(() => box.focus());

    rerender(ui(store, 't-b', search));

    // Let the switch's event and effects run before asserting nothing moved.
    await act(async () => {});
    expect(box).toHaveFocus();
  });

  it('does not take focus from an open modal', async () => {
    const store = makeStore();
    cacheThread(store, 't-a', [msg('1', 'user', 'first')]);
    cacheThread(store, 't-b', [msg('2', 'user', 'second')]);
    const modal = (
      <div role="dialog" aria-modal="true">
        <button type="button">Confirm</button>
      </div>
    );

    const { rerender } = render(ui(store, 't-a'));
    await waitFor(() => expect(composer()).toHaveFocus());

    rerender(ui(store, 't-a', modal));
    const confirm = screen.getByRole('button', { name: 'Confirm' });
    act(() => confirm.focus());

    rerender(ui(store, 't-b', modal));

    await act(async () => {});
    expect(confirm).toHaveFocus();
  });

  it('does not re-focus on new messages or re-renders of the same conversation', async () => {
    const store = makeStore();
    cacheThread(store, 't-same', [msg('1', 'user', 'q'), msg('2', 'agent', 'a')]);

    const { rerender } = render(ui(store, 't-same'));
    await waitFor(() => expect(composer()).toHaveFocus());
    blurAll();

    cacheThread(store, 't-same', [
      msg('1', 'user', 'q'),
      msg('2', 'agent', 'a'),
      msg('4', 'user', 'follow-up'),
    ]);
    rerender(ui(store, 't-same'));

    await act(async () => {});
    expect(document.activeElement).toBe(document.body);
  });
});

describe('composer focus while disabled', () => {
  function Disabled() {
    const messages: ThreadMessageLike[] = [
      { role: 'user', content: [{ type: 'text', text: 'hello' }], createdAt: new Date() },
    ];
    const runtime = useExternalStoreRuntime({
      messages,
      isDisabled: true,
      convertMessage: (m: ThreadMessageLike) => m,
      onNew: async () => {},
    });
    return (
      <AssistantRuntimeProvider runtime={runtime}>
        <Thread />
      </AssistantRuntimeProvider>
    );
  }

  it('leaves a disabled composer unfocused', async () => {
    render(<Disabled />);

    await act(async () => {});
    expect(document.activeElement).toBe(document.body);
  });
});
