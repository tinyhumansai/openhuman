import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { WorkflowProposal } from '../store/chatRuntimeSlice';
import { useWorkflowBuilderChat } from './useWorkflowBuilderChat';

const chatSend = vi.hoisted(() => vi.fn());
vi.mock('../services/chatService', () => ({ chatSend }));

// Socket is always "connected" for these tests (offline is exercised via the
// prompt bar's error rendering).
vi.mock('../store/socketSelectors', () => ({ selectSocketStatus: () => 'connected' }));

const dispatch = vi.hoisted(() => vi.fn());
const selectorState = vi.hoisted(() => ({
  activeThreadIds: {} as Record<string, true>,
  proposals: {} as Record<string, WorkflowProposal>,
}));
vi.mock('../store/hooks', () => ({
  useAppDispatch: () => dispatch,
  useAppSelector: (sel: (s: unknown) => unknown) =>
    sel({
      thread: { activeThreadIds: selectorState.activeThreadIds },
      chatRuntime: { pendingWorkflowProposalsByThread: selectorState.proposals },
    }),
}));

// Tag thread/chatRuntime action creators so the dispatch mock can special-case
// the two thunks that need `.unwrap()`.
vi.mock('../store/threadSlice', () => ({
  createNewThread: (labels: string[]) => ({ type: 'createNewThread', labels }),
  addMessageLocal: (p: unknown) => ({ type: 'addMessageLocal', p }),
  markThreadInferenceActive: (id: string) => ({ type: 'markActive', id }),
  clearThreadInferenceActive: (id: string) => ({ type: 'clearActive', id }),
}));
vi.mock('../store/chatRuntimeSlice', () => ({
  beginInferenceTurn: (p: unknown) => ({ type: 'begin', p }),
  clearRuntimeForThread: (p: unknown) => ({ type: 'clearRuntime', p }),
  clearWorkflowProposalForThread: (p: unknown) => ({ type: 'clearProposal', p }),
  setToolTimelineForThread: (p: unknown) => ({ type: 'timeline', p }),
}));

describe('useWorkflowBuilderChat', () => {
  beforeEach(() => {
    chatSend.mockReset().mockResolvedValue(undefined);
    selectorState.activeThreadIds = {};
    selectorState.proposals = {};
    dispatch.mockReset().mockImplementation((action: { type: string }) => {
      if (action.type === 'createNewThread') {
        return { unwrap: () => Promise.resolve({ id: 'builder-1' }) };
      }
      if (action.type === 'addMessageLocal') {
        return { unwrap: () => Promise.resolve(undefined) };
      }
      return undefined;
    });
  });

  it('creates a dedicated thread on first send and dispatches the turn there', async () => {
    const { result } = renderHook(() => useWorkflowBuilderChat());
    expect(result.current.threadId).toBeNull();

    await act(async () => {
      await result.current.send({ displayText: 'hi', prompt: 'DELEGATE PROMPT' });
    });

    // A dedicated "workflow-builder" thread was created and the turn sent there.
    expect(dispatch).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'createNewThread', labels: ['workflow-builder'] })
    );
    expect(chatSend).toHaveBeenCalledWith({ threadId: 'builder-1', message: 'DELEGATE PROMPT' });
    await waitFor(() => expect(result.current.threadId).toBe('builder-1'));
  });

  it('surfaces the proposal the runtime parsed onto the dedicated thread', async () => {
    const proposal: WorkflowProposal = {
      name: 'Digest',
      graph: { nodes: [], edges: [] },
      requireApproval: true,
      summary: { trigger: 'schedule', steps: [] },
    };
    selectorState.proposals = { 'builder-1': proposal };

    const { result } = renderHook(() => useWorkflowBuilderChat());
    await act(async () => {
      await result.current.send({ displayText: 'hi', prompt: 'PROMPT' });
    });
    await waitFor(() => expect(result.current.proposal).toEqual(proposal));
  });

  it('reuses the same dedicated thread across sends (creates it once)', async () => {
    const { result } = renderHook(() => useWorkflowBuilderChat());
    await act(async () => {
      await result.current.send({ displayText: 'one', prompt: 'P1' });
    });
    await act(async () => {
      await result.current.send({ displayText: 'two', prompt: 'P2' });
    });
    const createCalls = dispatch.mock.calls.filter(
      ([a]) => (a as { type: string }).type === 'createNewThread'
    );
    expect(createCalls).toHaveLength(1);
    expect(chatSend).toHaveBeenLastCalledWith({ threadId: 'builder-1', message: 'P2' });
  });
});
