import { describe, expect, it } from 'vitest';

import reducer, {
  markWorkflowProposalCompleted,
  markWorkflowProposalSaved,
  setWorkflowProposalForThread,
  type WorkflowProposal,
} from '../chatRuntimeSlice';

const proposal = (name: string, sourceMessageId?: string): WorkflowProposal => ({
  name,
  graph: { nodes: [], edges: [] },
  requireApproval: true,
  summary: { trigger: 'manual', steps: [] },
  ...(sourceMessageId ? { sourceMessageId } : {}),
});

const withPending = (p: WorkflowProposal) =>
  reducer(undefined, setWorkflowProposalForThread({ threadId: 't1', proposal: p }));

describe('markWorkflowProposalSaved', () => {
  it('records the saved flow on the proposal that was saved', () => {
    const state = reducer(
      withPending(proposal('Digest', 'm1')),
      markWorkflowProposalSaved({
        threadId: 't1',
        flowId: 'f1',
        name: 'Digest',
        sourceMessageId: 'm1',
      })
    );
    expect(state.pendingWorkflowProposalsByThread.t1?.savedFlowId).toBe('f1');
  });

  it('leaves a newer proposal that replaced it untouched', () => {
    const state = reducer(
      withPending(proposal('Digest v2', 'm2')),
      markWorkflowProposalSaved({
        threadId: 't1',
        flowId: 'f1',
        name: 'Digest',
        sourceMessageId: 'm1',
      })
    );
    expect(state.pendingWorkflowProposalsByThread.t1?.savedFlowId).toBeUndefined();
  });
});

describe('markWorkflowProposalCompleted', () => {
  it('ignores a late completion from a proposal that was replaced', () => {
    const state = reducer(
      withPending(proposal('Digest', 'm2')),
      markWorkflowProposalCompleted({
        threadId: 't1',
        flowId: 'f1',
        name: 'Digest',
        sourceMessageId: 'm1',
      })
    );
    expect(state.pendingWorkflowProposalsByThread.t1?.completedFlowId).toBeUndefined();
  });
});
