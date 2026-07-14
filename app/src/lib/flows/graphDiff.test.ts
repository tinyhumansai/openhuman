import { describe, expect, it } from 'vitest';

import { buildPreviewGraph, diffGraphs } from './graphDiff';
import type { WorkflowGraph, WorkflowNode } from './types';

function node(id: string): WorkflowNode {
  return { id, kind: 'agent', name: id, config: {}, ports: [] };
}

function graph(ids: string[], edges: WorkflowGraph['edges'] = []): WorkflowGraph {
  return { schema_version: 1, name: 'g', nodes: ids.map(node), edges };
}

describe('diffGraphs', () => {
  it('reports added and removed node ids by id', () => {
    const current = graph(['a', 'b']);
    const proposed = graph(['b', 'c']);
    const d = diffGraphs(current, proposed);
    expect([...d.addedNodeIds]).toEqual(['c']);
    expect([...d.removedNodeIds]).toEqual(['a']);
    expect(d.hasChanges).toBe(true);
  });

  it('reports no changes when the node set is identical', () => {
    const d = diffGraphs(graph(['a', 'b']), graph(['b', 'a']));
    expect(d.addedNodeIds.size).toBe(0);
    expect(d.removedNodeIds.size).toBe(0);
    expect(d.hasChanges).toBe(false);
  });

  it('is safe against null/undefined graphs', () => {
    expect(diffGraphs(null, graph(['a'])).addedNodeIds).toEqual(new Set(['a']));
    expect(diffGraphs(graph(['a']), null).removedNodeIds).toEqual(new Set(['a']));
    expect(diffGraphs(null, null).hasChanges).toBe(false);
  });
});

describe('buildPreviewGraph', () => {
  it('returns the proposed graph unchanged when nothing is removed', () => {
    const proposed = graph(['a', 'c']);
    expect(buildPreviewGraph(graph(['a']), proposed, new Set())).toBe(proposed);
  });

  it('carries removed nodes (and their edges) over as ghosts', () => {
    const current = graph(
      ['a', 'b'],
      [{ from_node: 'a', from_port: 'main', to_node: 'b', to_port: 'main' }]
    );
    const proposed = graph(['a', 'c']);
    const preview = buildPreviewGraph(current, proposed, new Set(['b']));
    expect(preview.nodes.map(n => n.id).sort()).toEqual(['a', 'b', 'c']);
    // The a→b edge is preserved so the ghosted node still shows its wiring.
    expect(preview.edges).toContainEqual({
      from_node: 'a',
      from_port: 'main',
      to_node: 'b',
      to_port: 'main',
    });
  });
});
