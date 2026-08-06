import { describe, expect, it } from 'vitest';

import type { ThreadMessage } from '../../types/thread';
import {
  coerceWorkflowProposal,
  extractWorkflowProposalFromMessages,
  maybeParseWorkflowProposalTool,
  parseWorkflowProposal,
} from './workflowProposal';

const proposalPayload = (name: string) => ({
  type: 'workflow_proposal',
  persisted: false,
  name,
  graph: { nodes: [], edges: [] },
  require_approval: true,
  summary: {
    trigger: 'schedule: 0 9 * * *',
    steps: [{ kind: 'tool_call', name: 'Fetch trending tweets', config_hint: 'TWITTER_SEARCH' }],
  },
});

const message = (
  id: string,
  extraMetadata: Record<string, unknown>,
  overrides: Partial<ThreadMessage> = {}
): ThreadMessage => ({
  id,
  content: 'Workflow proposal ready',
  type: 'text',
  extraMetadata,
  sender: 'agent',
  createdAt: '2026-07-21T06:00:00Z',
  ...overrides,
});

describe('parseWorkflowProposal / coerceWorkflowProposal', () => {
  it('parses a valid proposal payload with summary steps', () => {
    const proposal = parseWorkflowProposal(JSON.stringify(proposalPayload('Daily X Trending')));
    expect(proposal).not.toBeNull();
    expect(proposal?.name).toBe('Daily X Trending');
    expect(proposal?.requireApproval).toBe(true);
    expect(proposal?.summary.trigger).toBe('schedule: 0 9 * * *');
    expect(proposal?.summary.steps).toEqual([
      { kind: 'tool_call', name: 'Fetch trending tweets', config_hint: 'TWITTER_SEARCH' },
    ]);
  });

  it('defaults requireApproval to true unless explicitly false', () => {
    const explicit = coerceWorkflowProposal({ ...proposalPayload('X'), require_approval: false });
    expect(explicit?.requireApproval).toBe(false);
    const omitted = coerceWorkflowProposal({ type: 'workflow_proposal', name: 'X', graph: {} });
    expect(omitted?.requireApproval).toBe(true);
  });

  it('normalizes invalid summary steps without rejecting the proposal', () => {
    const proposal = coerceWorkflowProposal({
      ...proposalPayload('Mixed steps'),
      summary: {
        trigger: 42,
        steps: [
          null,
          'not an object',
          { kind: 7, name: false, config_hint: ['not', 'a', 'string'] },
        ],
      },
    });

    expect(proposal?.summary).toEqual({
      trigger: '',
      steps: [{ kind: 'unknown', name: '', config_hint: undefined }],
    });
  });

  it('rejects non-proposal payloads, malformed JSON, and missing fields', () => {
    expect(parseWorkflowProposal('not json')).toBeNull();
    expect(coerceWorkflowProposal(null)).toBeNull();
    expect(coerceWorkflowProposal({ type: 'something_else' })).toBeNull();
    expect(coerceWorkflowProposal({ type: 'workflow_proposal', graph: {} })).toBeNull();
    expect(coerceWorkflowProposal({ type: 'workflow_proposal', name: 'x' })).toBeNull();
  });
});

describe('maybeParseWorkflowProposalTool', () => {
  it('is content-based: any tool name with a proposal-shaped output matches', () => {
    const output = JSON.stringify(proposalPayload('Via Edit Tool'));
    expect(maybeParseWorkflowProposalTool('edit_workflow', true, output)?.name).toBe(
      'Via Edit Tool'
    );
    expect(maybeParseWorkflowProposalTool('totally_new_tool', true, output)?.name).toBe(
      'Via Edit Tool'
    );
  });

  it('ignores failed calls and empty output', () => {
    const output = JSON.stringify(proposalPayload('X'));
    expect(maybeParseWorkflowProposalTool('propose_workflow', false, output)).toBeNull();
    expect(maybeParseWorkflowProposalTool('propose_workflow', true, undefined)).toBeNull();
  });
});

describe('extractWorkflowProposalFromMessages', () => {
  it('rehydrates the newest unconsumed proposal and tags its source message', () => {
    const messages = [
      message('m1', { scope: 'workflow_proposal', proposal: proposalPayload('Old Draft') }),
      message('m2', {}),
      message('m3', { scope: 'workflow_proposal', proposal: proposalPayload('Latest Draft') }),
    ];
    const proposal = extractWorkflowProposalFromMessages(messages);
    expect(proposal?.name).toBe('Latest Draft');
    expect(proposal?.sourceMessageId).toBe('m3');
  });

  it('skips consumed proposals so a saved/dismissed card does not resurrect', () => {
    const messages = [
      message('m1', { scope: 'workflow_proposal', proposal: proposalPayload('Kept') }),
      message('m2', {
        scope: 'workflow_proposal',
        proposal: proposalPayload('Already Saved'),
        consumed: true,
      }),
    ];
    const proposal = extractWorkflowProposalFromMessages(messages);
    expect(proposal?.name).toBe('Kept');
    expect(proposal?.sourceMessageId).toBe('m1');
  });

  it('returns null when there is no proposal metadata or the payload is malformed', () => {
    expect(extractWorkflowProposalFromMessages([])).toBeNull();
    expect(extractWorkflowProposalFromMessages([message('m1', {})])).toBeNull();
    expect(
      extractWorkflowProposalFromMessages([
        message('m1', { scope: 'workflow_proposal', proposal: { type: 'wrong' } }),
      ])
    ).toBeNull();
  });
});
