/**
 * Unit tests for `createBlankWorkflowGraph` (Phase 4a "start from scratch").
 * Locks the starter-graph shape the chooser persists: a single `manual`
 * trigger, no edges, and structural validity (one trigger, unique ids).
 */
import { describe, expect, it } from 'vitest';

import {
  BLANK_TRIGGER_NODE_ID,
  createBlankWorkflowGraph,
  deriveWorkflowName,
  MAX_DERIVED_NAME_LENGTH,
} from './newFlow';

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

describe('deriveWorkflowName', () => {
  it('uses the first line, collapsing whitespace', () => {
    expect(deriveWorkflowName('  digest   my Slack \nand more detail', 'fallback')).toBe(
      'digest my Slack'
    );
  });

  it('truncates long descriptions with an ellipsis', () => {
    const long = 'a'.repeat(2 * MAX_DERIVED_NAME_LENGTH);
    const name = deriveWorkflowName(long, 'fallback');
    expect(name.length).toBeLessThanOrEqual(MAX_DERIVED_NAME_LENGTH);
    expect(name.endsWith('…')).toBe(true);
  });

  it('falls back when the description is blank', () => {
    expect(deriveWorkflowName('   \n whatever', 'New workflow')).toBe('New workflow');
    expect(deriveWorkflowName('', 'New workflow')).toBe('New workflow');
  });
});
