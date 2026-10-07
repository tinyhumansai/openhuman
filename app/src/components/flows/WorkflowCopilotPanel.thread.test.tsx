/**
 * The Workflow Copilot against the REAL assistant-ui `Thread`.
 *
 * `WorkflowCopilotPanel.test.tsx` stubs `Thread` to focus on the authoring
 * footer, and `WorkflowCopilotPanel.assistantUiRuntime.test.tsx` stubs it with
 * a runtime probe. This file mounts the real thing to pin down what those two
 * cannot: the copilot's transcript is the assistant-ui transcript of its OWN
 * builder thread, and the authoring footer lands in the Thread's `Composer`
 * slot in place of the chat composer (no model pill, no home-chat starters).
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it, vi } from 'vitest';

import type { WorkflowGraph } from '../../lib/flows/types';
import chatRuntimeReducer from '../../store/chatRuntimeSlice';
import threadReducer from '../../store/threadSlice';
import type { ThreadMessage } from '../../types/thread';
import WorkflowCopilotPanel from './WorkflowCopilotPanel';

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));
vi.mock('../../services/coreRpcClient', async importOriginal => ({
  ...(await importOriginal<typeof import('../../services/coreRpcClient')>()),
  callCoreRpc: vi.fn(),
}));

const hookState = vi.hoisted(() => ({
  threadId: 'builder-1' as string | null,
  sending: false,
  proposal: null,
  pendingApproval: null,
  capped: false,
  error: null as string | null,
  send: vi.fn(),
  stop: vi.fn(),
  clearProposal: vi.fn(),
}));
vi.mock('../../hooks/useWorkflowBuilderChat', () => ({ useWorkflowBuilderChat: () => hookState }));

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

const graph: WorkflowGraph = { schema_version: 1, name: 'g', nodes: [], edges: [] };

function renderPanel(builderMessages: ThreadMessage[]) {
  const store = configureStore({
    reducer: combineReducers({ thread: threadReducer, chatRuntime: chatRuntimeReducer }),
    preloadedState: {
      thread: {
        threads: [],
        selectedThreadId: 't-home',
        activeThreadIds: {},
        welcomeThreadId: null,
        messagesByThreadId: {
          't-home': [msg('h1', 'user', 'home chat message')],
          'builder-1': builderMessages,
        },
        messages: [],
        isLoadingThreads: false,
        isLoadingMessages: false,
        messagesError: null,
      },
    } as never,
  });
  return render(
    <Provider store={store}>
      <WorkflowCopilotPanel
        graph={graph}
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
      />
    </Provider>
  );
}

describe('WorkflowCopilotPanel on the real assistant-ui Thread', () => {
  it('renders the builder thread through assistant-ui, not the home thread', () => {
    renderPanel([
      msg('b1', 'user', 'add a slack step'),
      msg('b2', 'agent', 'Proposed a Slack notification step.'),
    ]);

    const viewport = document.querySelector('[data-slot="aui_thread-viewport"]');
    expect(viewport).not.toBeNull();
    expect(viewport).toHaveTextContent('add a slack step');
    expect(viewport).toHaveTextContent('Proposed a Slack notification step.');
    expect(viewport).not.toHaveTextContent('home chat message');
    // The settled reply is an assistant-ui message, not a legacy bubble.
    expect(screen.getByTestId('agent-message')).toBeInTheDocument();
  });

  it('puts the builder composer in the Composer slot instead of the chat composer', () => {
    renderPanel([]);

    // The copilot's empty hint is the Thread's welcome.
    expect(screen.getByTestId('workflow-copilot-empty')).toBeInTheDocument();
    // The builder composer is present...
    expect(screen.getByPlaceholderText('flows.copilot.placeholder')).toBeInTheDocument();
    // ...and the chat composer shell (with its model pill) is not.
    expect(document.querySelector('[data-slot="aui_composer-shell"]')).toBeNull();
    // No home-chat starter prompts: clicking one would send it to the builder.
    expect(document.querySelector('.aui-thread-welcome-suggestions')).toBeNull();
    expect(document.querySelector('.aui-thread-followup-suggestions')).toBeNull();
  });
});
