import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { BuilderTurnResult } from '../services/api/flowsApi';
import type { WorkflowProposal } from '../store/chatRuntimeSlice';
import type { ThreadMessage } from '../types/thread';
import { useWorkflowBuilderChat } from './useWorkflowBuilderChat';

// The hook now runs the builder server-side via `openhuman.flows_build`.
const buildWorkflow = vi.hoisted(() => vi.fn());
vi.mock('../services/api/flowsApi', () => ({ buildWorkflow }));

// Socket is always "connected" for these tests.
vi.mock('../store/socketSelectors', () => ({ selectSocketStatus: () => 'connected' }));

const dispatch = vi.hoisted(() => vi.fn());
const selectorState = vi.hoisted(() => ({
  proposals: {} as Record<string, WorkflowProposal>,
  messagesByThreadId: {} as Record<string, unknown[]>,
  toolTimelineByThread: {} as Record<string, unknown[]>,
  streamingAssistantByThread: {} as Record<string, { content: string }>,
}));
vi.mock('../store/hooks', () => ({
  useAppDispatch: () => dispatch,
  useAppSelector: (sel: (s: unknown) => unknown) =>
    sel({
      thread: { messagesByThreadId: selectorState.messagesByThreadId },
      chatRuntime: {
        pendingWorkflowProposalsByThread: selectorState.proposals,
        toolTimelineByThread: selectorState.toolTimelineByThread,
        streamingAssistantByThread: selectorState.streamingAssistantByThread,
      },
    }),
}));

const THREAD_NOT_FOUND_MESSAGE = vi.hoisted(() => 'This thread is no longer available.');
vi.mock('../store/threadSlice', () => ({
  createNewThread: (labels: string[]) => ({ type: 'createNewThread', labels }),
  addMessageLocal: (p: unknown) => ({ type: 'addMessageLocal', p }),
  loadThreadMessages: (threadId: string) => ({ type: 'loadThreadMessages', threadId }),
  THREAD_NOT_FOUND_MESSAGE,
}));
vi.mock('../store/chatRuntimeSlice', () => ({
  clearWorkflowProposalForThread: (p: unknown) => ({ type: 'clearProposal', p }),
  setWorkflowProposalForThread: (p: unknown) => ({ type: 'setProposal', p }),
  fetchAndHydrateTurnState: (threadId: string) => ({ type: 'fetchAndHydrateTurnState', threadId }),
  fetchAndHydrateTurnHistory: (threadId: string) => ({
    type: 'fetchAndHydrateTurnHistory',
    threadId,
  }),
}));

// The hook reads the live store directly (not the stale closed-over selector
// value) to dedup against a message the streamed `chat_done` path may have
// already appended for this exact turn — see the `assistantText` fallback
// branch. Controllable per test via `rawStoreState.thread.messagesByThreadId`.
const rawStoreState = vi.hoisted(() => ({
  thread: { messagesByThreadId: {} as Record<string, { sender: string; content: string }[]> },
}));
vi.mock('../store', () => ({ store: { getState: () => rawStoreState } }));

const okResult = (over: Partial<BuilderTurnResult> = {}): BuilderTurnResult => ({
  proposal: null,
  assistantText: 'done',
  error: null,
  ...over,
});

describe('useWorkflowBuilderChat', () => {
  beforeEach(() => {
    buildWorkflow.mockReset().mockResolvedValue(okResult());
    selectorState.proposals = {};
    selectorState.messagesByThreadId = {};
    selectorState.toolTimelineByThread = {};
    selectorState.streamingAssistantByThread = {};
    rawStoreState.thread.messagesByThreadId = {};
    dispatch.mockReset().mockImplementation((action: { type: string }) => {
      if (action.type === 'createNewThread') {
        return { unwrap: () => Promise.resolve({ id: 'builder-1' }) };
      }
      if (action.type === 'addMessageLocal') {
        return { unwrap: () => Promise.resolve(undefined) };
      }
      if (action.type === 'loadThreadMessages') {
        // Default: fetch succeeds (mirrors the real thunk's fulfilled action).
        return Promise.resolve({ type: 'loadThreadMessages/fulfilled', payload: { messages: [] } });
      }
      return undefined;
    });
  });

  it('creates a dedicated thread on first send and runs the builder there', async () => {
    const { result } = renderHook(() => useWorkflowBuilderChat());
    expect(result.current.threadId).toBeNull();

    await act(async () => {
      await result.current.send({
        displayText: 'hi',
        request: { mode: 'create', instruction: 'email me a digest' },
      });
    });

    // A dedicated "workflow-builder" thread was created and the agent run there.
    expect(dispatch).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'createNewThread', labels: ['workflow-builder'] })
    );
    // The builder turn streams onto the dedicated thread — its id is threaded
    // into `flows_build` as the second arg.
    expect(buildWorkflow).toHaveBeenCalledWith(
      { mode: 'create', instruction: 'email me a digest' },
      'builder-1'
    );
    await waitFor(() => expect(result.current.threadId).toBe('builder-1'));
  });

  it('surfaces the proposal the builder returned by dispatching it into the store', async () => {
    const proposal: WorkflowProposal = {
      name: 'Digest',
      graph: { nodes: [], edges: [] },
      requireApproval: true,
      summary: { trigger: 'schedule', steps: [] },
    };
    buildWorkflow.mockResolvedValue(okResult({ proposal }));

    const { result } = renderHook(() => useWorkflowBuilderChat());
    await act(async () => {
      await result.current.send({
        displayText: 'hi',
        request: { mode: 'create', instruction: 'x' },
      });
    });

    // The proposal is written into the shared store slice via setProposal.
    expect(dispatch).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'setProposal', p: { threadId: 'builder-1', proposal } })
    );
  });

  it('appends the user turn locally — the runtime normally owns the agent reply', async () => {
    // Simulate the streamed path already having delivered this exact text via
    // `chat_done` (the normal case when streaming is wired) so the fallback
    // branch below can prove it does NOT double the bubble.
    rawStoreState.thread.messagesByThreadId = {
      'builder-1': [{ sender: 'agent', content: 'Here is your workflow.' }],
    };
    buildWorkflow.mockResolvedValue(okResult({ assistantText: 'Here is your workflow.' }));
    const { result } = renderHook(() => useWorkflowBuilderChat());
    await act(async () => {
      await result.current.send({
        displayText: 'hi',
        request: { mode: 'create', instruction: 'x' },
      });
    });
    const appended = dispatch.mock.calls
      .map(([a]) => a as { type: string; p?: { message?: { sender?: string } } })
      .filter(a => a.type === 'addMessageLocal');
    // The web channel never persists user messages, so the hook appends the
    // user turn itself...
    expect(appended.some(a => a.p?.message?.sender === 'user')).toBe(true);
    // ...but NOT the agent reply when it was already streamed — appending
    // here too would double it.
    expect(appended.some(a => a.p?.message?.sender === 'agent')).toBe(false);
  });

  it('surfaces a clarifying question as an assistant message when the builder returns plain text with no proposal (fallback)', async () => {
    buildWorkflow.mockResolvedValue(
      okResult({
        proposal: null,
        error: null,
        assistantText: 'Which Slack channel — #eng or #sales?',
      })
    );
    const { result } = renderHook(() => useWorkflowBuilderChat());
    await act(async () => {
      await result.current.send({
        displayText: 'post a daily summary to slack',
        request: { mode: 'create', instruction: 'post a daily summary to slack' },
      });
    });

    const appendedAgentMessages = dispatch.mock.calls
      .map(([a]) => a as { type: string; p?: { threadId?: string; message?: ThreadMessage } })
      .filter(a => a.type === 'addMessageLocal' && a.p?.message?.sender === 'agent');
    expect(appendedAgentMessages).toHaveLength(1);
    expect(appendedAgentMessages[0]?.p?.message?.content).toBe(
      'Which Slack channel — #eng or #sales?'
    );
    expect(appendedAgentMessages[0]?.p?.threadId).toBe('builder-1');
    // No proposal was surfaced for this turn.
    expect(dispatch.mock.calls.some(([a]) => (a as { type: string }).type === 'setProposal')).toBe(
      false
    );
  });

  it('does not double-append when a proposal is returned alongside assistant text', async () => {
    const proposal: WorkflowProposal = {
      name: 'Digest',
      graph: { nodes: [], edges: [] },
      requireApproval: true,
      summary: { trigger: 'schedule', steps: [] },
    };
    buildWorkflow.mockResolvedValue(
      okResult({ proposal, assistantText: "I've built this — review below." })
    );
    const { result } = renderHook(() => useWorkflowBuilderChat());
    await act(async () => {
      await result.current.send({
        displayText: 'hi',
        request: { mode: 'create', instruction: 'x' },
      });
    });

    // A proposal result still sets the proposal, unchanged...
    expect(dispatch).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'setProposal', p: { threadId: 'builder-1', proposal } })
    );
    // ...and does NOT also append an agent chat message (the proposal branch
    // is exclusive of the assistant-text fallback branch).
    expect(
      dispatch.mock.calls.some(
        ([a]) =>
          (a as { type: string; p?: { message?: { sender?: string } } }).type ===
            'addMessageLocal' &&
          (a as { p?: { message?: { sender?: string } } }).p?.message?.sender === 'agent'
      )
    ).toBe(false);
  });

  it('reuses the same dedicated thread across sends (creates it once)', async () => {
    const { result } = renderHook(() => useWorkflowBuilderChat());
    await act(async () => {
      await result.current.send({
        displayText: 'one',
        request: { mode: 'create', instruction: 'a' },
      });
    });
    await act(async () => {
      await result.current.send({
        displayText: 'two',
        request: { mode: 'revise', instruction: 'b' },
      });
    });
    const createCalls = dispatch.mock.calls.filter(
      ([a]) => (a as { type: string }).type === 'createNewThread'
    );
    expect(createCalls).toHaveLength(1);
    expect(buildWorkflow).toHaveBeenLastCalledWith(
      { mode: 'revise', instruction: 'b' },
      'builder-1'
    );
  });

  it('surfaces the streamed tool timeline + live response for the dedicated thread', async () => {
    const { result } = renderHook(() => useWorkflowBuilderChat());
    await act(async () => {
      await result.current.send({
        displayText: 'hi',
        request: { mode: 'create', instruction: 'x' },
      });
    });
    // Simulate the runtime streaming onto this thread, then re-render.
    selectorState.toolTimelineByThread = {
      'builder-1': [{ id: 't1', name: 'propose_workflow', round: 0, status: 'running' }],
    };
    selectorState.streamingAssistantByThread = { 'builder-1': { content: 'drafting…' } };
    const { result: result2 } = renderHook(() => useWorkflowBuilderChat('builder-1'));
    expect(result2.current.toolTimeline).toHaveLength(1);
    expect(result2.current.liveResponse).toBe('drafting…');
  });

  it('sets an error when the builder run fails without a proposal', async () => {
    buildWorkflow.mockResolvedValue(okResult({ error: 'run failed', assistantText: '' }));
    const { result } = renderHook(() => useWorkflowBuilderChat());
    await act(async () => {
      await result.current.send({
        displayText: 'hi',
        request: { mode: 'create', instruction: 'x' },
      });
    });
    await waitFor(() => expect(result.current.error).toBe('run failed'));
  });

  describe('displayMessages', () => {
    it('excludes isInterim agent messages but keeps user + terminal agent messages (incl. a clarifying question)', () => {
      selectorState.messagesByThreadId = {
        'builder-1': [
          { id: 'm1', sender: 'user', content: 'build me a digest', extraMetadata: {} },
          {
            id: 'm2',
            sender: 'agent',
            content: 'Let me check your calendar first.',
            extraMetadata: { isInterim: true, requestId: 'r1' },
          },
          {
            id: 'm3',
            sender: 'agent',
            content: 'Now let me build the workflow.',
            extraMetadata: { isInterim: true, requestId: 'r1' },
          },
          // The #4630-style clarifying question is appended via
          // `addMessageLocal` (the `send` fallback branch), never through
          // `onInterim` — so it carries no `isInterim` tag and must still
          // render as a bubble.
          {
            id: 'm4',
            sender: 'agent',
            content: 'Which Slack channel — #eng or #sales?',
            extraMetadata: {},
          },
        ] as ThreadMessage[],
      };
      const { result } = renderHook(() => useWorkflowBuilderChat('builder-1'));
      expect(result.current.messages).toHaveLength(4);
      expect(result.current.displayMessages.map(m => m.id)).toEqual(['m1', 'm4']);
    });
  });

  describe('rehydration on mount with seedThreadId', () => {
    it('dispatches loadThreadMessages + turn state/history rehydration for the seed thread', () => {
      renderHook(() => useWorkflowBuilderChat('seed-thread-1'));
      expect(dispatch).toHaveBeenCalledWith(
        expect.objectContaining({ type: 'loadThreadMessages', threadId: 'seed-thread-1' })
      );
      expect(dispatch).toHaveBeenCalledWith(
        expect.objectContaining({ type: 'fetchAndHydrateTurnState', threadId: 'seed-thread-1' })
      );
      expect(dispatch).toHaveBeenCalledWith(
        expect.objectContaining({ type: 'fetchAndHydrateTurnHistory', threadId: 'seed-thread-1' })
      );
    });

    it('does not rehydrate when mounted with no seed thread', () => {
      renderHook(() => useWorkflowBuilderChat());
      expect(dispatch).not.toHaveBeenCalledWith(
        expect.objectContaining({ type: 'loadThreadMessages' })
      );
    });

    it('does not rehydrate a thread this hook just created when seedThreadId echoes it back', async () => {
      // Mirrors the real wiring: `WorkflowCopilotPanel` reports every
      // `threadId` change back up via `onThreadIdChange`, and `FlowCanvasPage`
      // re-passes that straight back in as `seedThreadId` on the next render.
      // A first send() creates a fresh thread; the resulting re-render must
      // NOT trigger a rehydrate for it — that would race the in-flight turn
      // and `loadThreadMessages.fulfilled` would wipe the just-appended
      // message(s) back out of the transcript.
      const { result, rerender } = renderHook(
        ({ seedThreadId }: { seedThreadId?: string | null }) =>
          useWorkflowBuilderChat(seedThreadId),
        { initialProps: { seedThreadId: undefined as string | null | undefined } }
      );

      await act(async () => {
        await result.current.send({
          displayText: 'hi',
          request: { mode: 'create', instruction: 'x' },
        });
      });
      expect(result.current.threadId).toBe('builder-1');

      dispatch.mockClear();
      rerender({ seedThreadId: 'builder-1' });

      expect(dispatch).not.toHaveBeenCalledWith(
        expect.objectContaining({ type: 'loadThreadMessages' })
      );
      expect(dispatch).not.toHaveBeenCalledWith(
        expect.objectContaining({ type: 'fetchAndHydrateTurnState' })
      );
      expect(dispatch).not.toHaveBeenCalledWith(
        expect.objectContaining({ type: 'fetchAndHydrateTurnHistory' })
      );
    });

    it('still rehydrates a genuinely pre-existing seed thread this hook did not create', () => {
      // A different thread id than any this hook instance created via
      // send() — the normal "resume a persisted copilot thread on mount" case
      // — must still rehydrate.
      renderHook(() => useWorkflowBuilderChat('previously-persisted-thread'));
      expect(dispatch).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'loadThreadMessages',
          threadId: 'previously-persisted-thread',
        })
      );
    });

    it('clears threadId when the seeded thread no longer exists (stale persisted id)', async () => {
      // `loadThreadMessages` rejects with THREAD_NOT_FOUND_MESSAGE when the
      // cached/seeded thread was deleted server-side (see threadSlice.ts).
      // The hook must null out its `threadId` so a caller's onThreadIdChange
      // effect (WorkflowCopilotPanel -> FlowCanvasPage) clears the stale
      // `workflowCopilotThreads.ts` cache entry and the next send() creates a
      // fresh thread instead of retrying the dead one forever.
      dispatch.mockImplementation((action: { type: string }) => {
        if (action.type === 'createNewThread') {
          return { unwrap: () => Promise.resolve({ id: 'builder-1' }) };
        }
        if (action.type === 'addMessageLocal') {
          return { unwrap: () => Promise.resolve(undefined) };
        }
        if (action.type === 'loadThreadMessages') {
          return Promise.resolve({
            type: 'loadThreadMessages/rejected',
            payload: THREAD_NOT_FOUND_MESSAGE,
          });
        }
        return undefined;
      });

      const { result } = renderHook(() => useWorkflowBuilderChat('stale-thread'));
      expect(result.current.threadId).toBe('stale-thread');

      await waitFor(() => expect(result.current.threadId).toBeNull());
    });
  });

  describe('recovering from a stale thread id during send()', () => {
    it('clears threadId when addMessageLocal fails because the seeded thread was deleted', async () => {
      // Mirrors a stale `workflowCopilotThreads.ts` seed surviving past mount
      // (e.g. the rehydrate GET raced and lost, or the thread was deleted
      // between mount and this send). `addMessageLocal` rejects with
      // THREAD_NOT_FOUND_MESSAGE (see threadSlice.ts); the hook must recover
      // by nulling `threadId` so the NEXT send creates a fresh thread instead
      // of erroring forever against the dead one.
      dispatch.mockImplementation((action: { type: string }) => {
        if (action.type === 'addMessageLocal') {
          return { unwrap: () => Promise.reject(THREAD_NOT_FOUND_MESSAGE) };
        }
        if (action.type === 'loadThreadMessages') {
          return Promise.resolve({
            type: 'loadThreadMessages/fulfilled',
            payload: { messages: [] },
          });
        }
        return undefined;
      });

      const { result } = renderHook(() => useWorkflowBuilderChat('stale-thread'));
      expect(result.current.threadId).toBe('stale-thread');

      await act(async () => {
        await result.current.send({
          displayText: 'hi',
          request: { mode: 'create', instruction: 'x' },
        });
      });

      expect(result.current.error).toBe(THREAD_NOT_FOUND_MESSAGE);
      expect(result.current.threadId).toBeNull();
    });
  });
});
