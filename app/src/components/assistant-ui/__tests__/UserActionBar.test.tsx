/**
 * The user-message action bar offers only what the runtime can honour (#5897).
 *
 * # Why this exists next to the browser spec
 *
 * `chat-user-message-edit-affordance.spec.ts` is the primary guard: it asserts
 * the rendered DOM in Chromium, which is where the dead Edit button actually
 * reached users, and it is revert-proofed. This file is the jsdom half, for one
 * specific reason — Playwright output does not feed `diff-cover`, which reads
 * only Vitest and cargo-llvm-cov, so without it the capability gate counts as
 * an uncovered changed line on the merge gate.
 *
 * # Why it mounts `Thread` rather than `UserActionBar`
 *
 * `UserActionBar` cannot be rendered on its own: `ActionBarPrimitive.Root`
 * needs an assistant-ui message scope and throws
 * "The current scope does not have a `message` property" without one. Exporting
 * the component to get at it would widen the module surface for a test and
 * still not supply the scope. Seeding one user message into
 * `thread.messagesByThreadId` — which is exactly what
 * `useOpenHumanExternalStore` reads — gets the real render path instead.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it } from 'vitest';

import { AssistantUiRuntimeProvider } from '../../../providers/AssistantUiRuntimeProvider';
import chatRuntimeReducer from '../../../store/chatRuntimeSlice';
import threadReducer from '../../../store/threadSlice';
import type { ThreadMessage } from '../../../types/thread';
import { Thread } from '../thread';

const THREAD_ID = 't-action-bar';

const userMessage: ThreadMessage = {
  id: 'm-1',
  content: 'hover me',
  type: 'text',
  extraMetadata: {},
  sender: 'user',
  createdAt: new Date('2026-01-01T00:00:00.000Z').toISOString(),
};

function renderThreadWithOneUserMessage() {
  const store = configureStore({
    reducer: combineReducers({ thread: threadReducer, chatRuntime: chatRuntimeReducer }),
    preloadedState: {
      thread: {
        ...threadReducer(undefined, { type: '@@INIT' }),
        selectedThreadId: THREAD_ID,
        messagesByThreadId: { [THREAD_ID]: [userMessage] },
      },
    },
  });

  return render(
    <Provider store={store}>
      <AssistantUiRuntimeProvider threadId={THREAD_ID}>
        <Thread />
      </AssistantUiRuntimeProvider>
    </Provider>
  );
}

describe('User-message action bar — capability-gated Edit (#5897)', () => {
  it('renders the user message and its action bar', async () => {
    const { container } = renderThreadWithOneUserMessage();

    // The control, and what stops the next test being vacuous: if the message
    // or its action bar never rendered, "no Edit button" would pass for the
    // wrong reason.
    await waitFor(() => {
      expect(screen.getByText('hover me')).toBeInTheDocument();
    });
    expect(container.querySelector('.aui-user-action-bar-root')).not.toBeNull();
  });

  it('does not offer an Edit control while the runtime cannot edit', async () => {
    const { container } = renderThreadWithOneUserMessage();

    await waitFor(() => {
      expect(screen.getByText('hover me')).toBeInTheDocument();
    });

    // `useOpenHumanExternalStore` implements neither `onEdit` nor
    // `setMessages`, so assistant-ui reports `edit: false` and the gate must
    // withhold the button. Asserted by the class the product CSS and the
    // browser spec both key on.
    expect(container.querySelectorAll('.aui-user-action-edit')).toHaveLength(0);
  });
});
