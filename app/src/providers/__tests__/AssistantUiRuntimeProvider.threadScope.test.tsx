/**
 * The runtime must represent the thread it is GIVEN.
 *
 * `AssistantUiRuntimeProvider` used to read `state.thread.selectedThreadId`
 * itself, which is only correct for the home chat. `ChatThreadView` is also
 * rendered by the Workflow Copilot against its own dedicated builder thread,
 * which is never the selected one — so a runtime that follows the selection
 * would, once the transcript renders from assistant-ui primitives, paint the
 * home chat's messages inside the copilot.
 *
 * These tests state that as behaviour rather than as structure: what a probe
 * under the runtime sees, and what two simultaneously-mounted runtimes see.
 * Every one of them fails against the selection-reading version.
 */
import { useAui, useAuiState } from '@assistant-ui/react';
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { afterEach, describe, expect, it, vi } from 'vitest';

import chatRuntimeReducer from '../../store/chatRuntimeSlice';
import threadReducer from '../../store/threadSlice';
import type { ThreadMessage } from '../../types/thread';
import { AssistantUiRuntimeProvider } from '../AssistantUiRuntimeProvider';
import { __resetChatSurfaces, registerChatSurface } from '../chatSurfaceHandlers';

const HOME_THREAD = 't-home';
const COPILOT_THREAD = 't-copilot';

function msg(id: string, sender: 'user' | 'agent', content: string): ThreadMessage {
  return {
    id,
    sender,
    type: 'text',
    content,
    extraMetadata: {},
    createdAt: '2026-01-01T00:00:00.000Z',
  };
}

function buildStore() {
  return configureStore({
    reducer: combineReducers({ thread: threadReducer, chatRuntime: chatRuntimeReducer }),
    preloadedState: {
      thread: {
        threads: [],
        // The home chat's selection. The copilot's thread is deliberately NOT
        // this one — that difference is the whole point of the fixture.
        selectedThreadId: HOME_THREAD,
        activeThreadIds: {},
        welcomeThreadId: null,
        messagesByThreadId: {
          [HOME_THREAD]: [msg('h1', 'user', 'home question')],
          [COPILOT_THREAD]: [msg('c1', 'user', 'copilot question')],
        },
        messages: [],
        isLoadingThreads: false,
        isLoadingMessages: false,
        messagesError: null,
      },
    } as never,
  });
}

/** Renders what the NEAREST assistant-ui runtime believes the thread holds. */
function RuntimeProbe({ label }: { label: string }) {
  const thread = useAuiState(({ thread: t }) => t);
  return (
    <div data-testid={label}>
      {thread.messages
        .map(m => m.content.map(p => (p.type === 'text' ? p.text : '')).join(''))
        .join('|')}
    </div>
  );
}

afterEach(() => __resetChatSurfaces());

describe('AssistantUiRuntimeProvider thread scoping', () => {
  it('renders the thread it is given, not the selected one', () => {
    render(
      <Provider store={buildStore()}>
        <AssistantUiRuntimeProvider threadId={COPILOT_THREAD}>
          <RuntimeProbe label="probe" />
        </AssistantUiRuntimeProvider>
      </Provider>
    );
    expect(screen.getByTestId('probe')).toHaveTextContent('copilot question');
    expect(screen.getByTestId('probe')).not.toHaveTextContent('home question');
  });

  it('keeps two simultaneously-mounted runtimes from cross-contaminating', () => {
    render(
      <Provider store={buildStore()}>
        <AssistantUiRuntimeProvider>
          <RuntimeProbe label="home" />
          {/* Nested, exactly as the copilot panel nests inside the app-wide
              runtime that `ChatRuntimeProvider` mounts. */}
          <AssistantUiRuntimeProvider threadId={COPILOT_THREAD}>
            <RuntimeProbe label="copilot" />
          </AssistantUiRuntimeProvider>
        </AssistantUiRuntimeProvider>
      </Provider>
    );
    expect(screen.getByTestId('home')).toHaveTextContent('home question');
    expect(screen.getByTestId('copilot')).toHaveTextContent('copilot question');
  });

  it('treats an explicit null as "no thread", never as a request to follow the selection', () => {
    render(
      <Provider store={buildStore()}>
        <AssistantUiRuntimeProvider threadId={null}>
          <RuntimeProbe label="probe" />
        </AssistantUiRuntimeProvider>
      </Provider>
    );
    expect(screen.getByTestId('probe')).toHaveTextContent('');
  });

  it('follows the selection when no thread is given (the app-wide mount)', () => {
    render(
      <Provider store={buildStore()}>
        <AssistantUiRuntimeProvider>
          <RuntimeProbe label="probe" />
        </AssistantUiRuntimeProvider>
      </Provider>
    );
    expect(screen.getByTestId('probe')).toHaveTextContent('home question');
  });

  it('routes writes from each runtime to the surface owning its own thread', async () => {
    const homeSend = vi.fn(async () => {});
    const copilotSend = vi.fn(async () => {});
    registerChatSurface(HOME_THREAD, { send: homeSend });
    registerChatSurface(COPILOT_THREAD, { send: copilotSend });

    function Sender({ label }: { label: string }) {
      const aui = useAui();
      return (
        <button
          type="button"
          data-testid={label}
          onClick={() =>
            void aui.thread.append({ role: 'user', content: [{ type: 'text', text: label }] })
          }>
          send
        </button>
      );
    }

    render(
      <Provider store={buildStore()}>
        <AssistantUiRuntimeProvider>
          <Sender label="home" />
          <AssistantUiRuntimeProvider threadId={COPILOT_THREAD}>
            <Sender label="copilot" />
          </AssistantUiRuntimeProvider>
        </AssistantUiRuntimeProvider>
      </Provider>
    );

    await act(async () => {
      screen.getByTestId('copilot').click();
    });
    expect(copilotSend).toHaveBeenCalledWith('copilot');
    expect(homeSend).not.toHaveBeenCalled();

    await act(async () => {
      screen.getByTestId('home').click();
    });
    expect(homeSend).toHaveBeenCalledWith('home');
    expect(copilotSend).toHaveBeenCalledTimes(1);
  });
});
