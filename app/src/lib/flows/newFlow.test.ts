/**
 * Unit tests for `createBlankWorkflowGraph` (Phase 4a "start from scratch").
 * Locks the starter-graph shape the chooser persists: a single `manual`
 * trigger, no edges, and structural validity (one trigger, unique ids).
 */
import { describe, expect, it } from 'vitest';

import { BLANK_TRIGGER_NODE_ID, createBlankWorkflowGraph } from './newFlow';

describe('createBlankWorkflowGraph', () => {
  it('produces a single manual trigger and no edges', () => {
    const graph = createBlankWorkflowGraph('My flow', 'Trigger');
    expect(graph.schema_version).toBe(1);
    expect(graph.name).toBe('My flow');
    expect(graph.edges).toEqual([]);
    expect(graph.nodes).toHaveLength(1);

    const [trigger] = graph.nodes;
    expect(trigger.id).toBe(BLANK_TRIGGER_NODE_ID);
    expect(trigger.kind).toBe('trigger');
    expect(trigger.name).toBe('Trigger');
    expect(trigger.config).toEqual({ trigger_kind: 'manual' });
  });

  it('is structurally valid (exactly one trigger, unique ids)', () => {
    const graph = createBlankWorkflowGraph('x', 'y');
    const ids = graph.nodes.map(n => n.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(graph.nodes.filter(n => n.kind === 'trigger')).toHaveLength(1);
  });
});
