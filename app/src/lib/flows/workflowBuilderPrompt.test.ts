import { describe, expect, it } from 'vitest';

import type { WorkflowGraph } from './types';
import { buildCreatePrompt, buildRepairPrompt, buildRevisePrompt } from './workflowBuilderPrompt';

const graph: WorkflowGraph = {
  schema_version: 1,
  name: 'g',
  nodes: [{ id: 'a', kind: 'trigger', name: 'Start', config: {}, ports: [] }],
  edges: [],
};

describe('buildCreatePrompt', () => {
  it('includes the description and asks only for a proposal (never persist)', () => {
    const p = buildCreatePrompt('  email me new Slack messages  ');
    expect(p).toContain('email me new Slack messages');
    expect(p.toLowerCase()).toContain('workflow builder');
    expect(p.toLowerCase()).toContain('do not save');
  });
});

describe('buildRevisePrompt', () => {
  it('injects the current graph JSON and the instruction', () => {
    const p = buildRevisePrompt('add a Slack notification on failure', graph);
    expect(p).toContain('add a Slack notification on failure');
    expect(p).toContain(JSON.stringify(graph));
    expect(p.toLowerCase()).toContain('revise');
  });
});

describe('buildRepairPrompt', () => {
  it('references the run, error, and failing nodes and injects the graph', () => {
    const p = buildRepairPrompt({
      runId: 'run-9',
      error: 'HTTP 500 from webhook',
      failingNodeIds: ['n2'],
      graph,
    });
    expect(p).toContain('run-9');
    expect(p).toContain('get_flow_run');
    expect(p).toContain('HTTP 500 from webhook');
    expect(p).toContain('n2');
    expect(p).toContain(JSON.stringify(graph));
  });

  it('omits the error/nodes lines when absent', () => {
    const p = buildRepairPrompt({ runId: 'run-9', graph });
    expect(p).toContain('run-9');
    expect(p).not.toContain('Run error:');
    expect(p).not.toContain('Failing step node');
  });
});
