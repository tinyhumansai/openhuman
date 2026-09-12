/**
 * The Workflow Copilot's own assistant-ui runtime, end to end.
 *
 * Two properties are proven here, and both are about the copilot's DEDICATED
 * builder thread being a different thread from the home chat's selection:
 *
 * 1. READ — a component rendered where the transcript goes sees the copilot's
 *    messages, not the selected thread's. `ChatThreadView` is stubbed with a
 *    probe that uses assistant-ui's own hooks, which is precisely what the real
 *    transcript will do once it renders from `ThreadPrimitive`/`MessagePrimitive`.
 *    Against a runtime that reads `selectedThreadId` this probe shows the home
 *    chat's transcript inside the copilot — the regression this file exists for.
 *
 * 2. WRITE — appending through that runtime reaches the panel's REAL `submit`,
 *    which builds the structured `WorkflowBuilderSendParams` (build mode +
 *    current graph + flow id) and consumes the `WorkflowBuilderSendResult`. The
 *    only mock in that path is `useWorkflowBuilderChat` itself, i.e. the
 *    transport; the parameter construction and the `proposed` carry-forward
 *    under test are the panel's own code, so this is not a mock echoing itself.
 */
import { useAui, useAuiState } from '@assistant-ui/react';
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { WorkflowGraph, WorkflowNode } from '../../lib/flows/types';
import { __resetChatSurfaces, getChatSurface } from '../../providers/chatSurfaceHandlers';
import chatRuntimeReducer, { type WorkflowProposal } from '../../store/chatRuntimeSlice';
import threadReducer from '../../store/threadSlice';
import type { ThreadMessage } from '../../types/thread';
import WorkflowCopilotPanel from './WorkflowCopilotPanel';

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));
vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

// Stand-in for the transcript, deliberately written the way the migrated
// transcript will be written: it reads the runtime from context rather than
// taking a thread id. Whatever runtime the panel mounted is what it reports.
vi.mock('../../features/conversations/components/ChatThreadView', () => ({
  ChatThreadView: () => {
    const thread = useAuiState(({ thread: t }) => t);
    const aui = useAui();
    return (
      <div>
        <div data-testid="runtime-transcript">
          {thread.messages
            .map(m => m.content.map(p => (p.type === 'text' ? p.text : '')).join(''))
            .join('|')}
        </div>
        <button
          type="button"
          data-testid="runtime-append"
          onClick={() =>
            void aui.thread.append({
              role: 'user',
              content: [{ type: 'text', text: 'post a daily summary to slack' }],
            })
          }>
          append
        </button>
        <button
          type="button"
          data-testid="runtime-append-answer"
          onClick={() =>
            void aui.thread.append({ role: 'user', content: [{ type: 'text', text: '#eng' }] })
          }>
          answer
        </button>
      </div>
    );
  },
}));

const hookState = vi.hoisted(() => ({
  threadId: 'builder-1' as string | null,
  sending: false,
  turnActive: false,
  proposal: null as WorkflowProposal | null,
  pendingApproval: null,
  capped: false,
  error: null as string | null,
  messages: [] as unknown[],
  displayMessages: [] as unknown[],
  toolTimeline: [] as unknown[],
  liveResponse: '',
  send: vi.fn(),
  stop: vi.fn(),
  clearProposal: vi.fn(),
}));
vi.mock('../../hooks/useWorkflowBuilderChat', () => ({ useWorkflowBuilderChat: () => hookState }));

const HOME_THREAD = 't-home';
const BUILDER_THREAD = 'builder-1';

function msg(id: string, content: string): ThreadMessage {
  return {
    id,
    sender: 'user',
    type: 'text',
    content,
    extraMetadata: {},
    createdAt: '2026-01-01T00:00:00.000Z',
  };
}

function node(id: string): WorkflowNode {
  return { id, kind: 'agent', name: id, config: {}, ports: [] };
}
const baseGraph: WorkflowGraph = {
  schema_version: 1,
  name: 'g',
  nodes: [node('a'), node('b')],
  edges: [],
};

function buildStore() {
  return configureStore({
    reducer: combineReducers({ thread: threadReducer, chatRuntime: chatRuntimeReducer }),
    preloadedState: {
      thread: {
        threads: [],
        selectedThreadId: HOME_THREAD,
        activeThreadIds: {},
        welcomeThreadId: null,
        messagesByThreadId: {
          [HOME_THREAD]: [msg('h1', 'home chat message')],
          [BUILDER_THREAD]: [msg('b1', 'builder thread message')],
        },
        messages: [],
        isLoadingThreads: false,
        isLoadingMessages: false,
        messagesError: null,
      },
    } as never,
  });
}

function renderPanel() {
  return render(
    <Provider store={buildStore()}>
      <WorkflowCopilotPanel
        graph={baseGraph}
        flowId="flow-9"
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
      />
    </Provider>
  );
}

beforeEach(() => {
  hookState.threadId = BUILDER_THREAD;
  hookState.sending = false;
  hookState.proposal = null;
  hookState.capped = false;
  hookState.error = null;
  hookState.send = vi.fn().mockResolvedValue({ outcome: 'dispatched', proposed: false });
  hookState.stop = vi.fn();
});
afterEach(() => __resetChatSurfaces());

describe('WorkflowCopilotPanel assistant-ui runtime', () => {
  it('renders the copilot thread, not the selected home thread', () => {
    renderPanel();
    expect(screen.getByTestId('runtime-transcript')).toHaveTextContent('builder thread message');
    expect(screen.getByTestId('runtime-transcript')).not.toHaveTextContent('home chat message');
  });

  it('turns a runtime append into a structured builder turn carrying the build mode', async () => {
    renderPanel();
    await act(async () => {
      screen.getByTestId('runtime-append').click();
    });

    expect(hookState.send).toHaveBeenCalledTimes(1);
    const params = hookState.send.mock.calls[0][0] as {
      displayText: string;
      request: { mode: string; instruction: string; graph: unknown; flowId: string | null };
    };
    expect(params.displayText).toBe('post a daily summary to slack');
    expect(params.request.mode).toBe('revise');
    expect(params.request.instruction).toBe('post a daily summary to slack');
    expect(params.request.graph).toEqual(baseGraph);
    expect(params.request.flowId).toBe('flow-9');
  });

  it('consumes the send result: an unproposed turn carries its ask into the next one', async () => {
    hookState.send = vi
      .fn()
      .mockResolvedValueOnce({ outcome: 'dispatched', proposed: false })
      .mockResolvedValue({ outcome: 'dispatched', proposed: true });

    renderPanel();
    await act(async () => {
      screen.getByTestId('runtime-append').click();
    });
    await act(async () => {
      screen.getByTestId('runtime-append-answer').click();
    });

    expect(hookState.send).toHaveBeenCalledTimes(2);
    const second = hookState.send.mock.calls[1][0] as {
      displayText: string;
      request: { instruction: string };
    };
    // The user only typed the answer...
    expect(second.displayText).toBe('#eng');
    // ...but the unresolved ask from the `proposed: false` turn is prepended,
    // which is only possible if the first call's RESULT was consumed rather
    // than discarded by the bridge.
    expect(second.request.instruction).toContain('post a daily summary to slack');
    expect(second.request.instruction).toContain('#eng');
  });

  it('routes the runtime cancel to the builder hook stop', async () => {
    hookState.sending = true;
    renderPanel();
    // `sending` blocks a composer send, so `submit` no-ops; the cancel path is
    // the one under test here.
    await act(async () => {
      await getChatSurface(BUILDER_THREAD)?.cancel?.();
    });
    expect(hookState.stop).toHaveBeenCalledTimes(1);
  });
});
