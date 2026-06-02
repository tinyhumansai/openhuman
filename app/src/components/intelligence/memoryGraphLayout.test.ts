import { describe, expect, it } from 'vitest';

import type { GraphEdge, GraphNode } from '../../utils/tauriCommands';
import {
  buildGraph,
  CONTACT_COLOR,
  createSimulation,
  LEAF_COLOR,
  LEVEL_COLOR,
  levelColor,
  nodeColor,
  nodeGlows,
  nodeRadius,
  pickNode,
  type SimNode,
  supportsWebGL,
} from './memoryGraphLayout';

function summary(overrides: Partial<GraphNode> = {}): GraphNode {
  return { kind: 'summary', id: 's', label: 'S', level: 0, parent_id: null, ...overrides };
}
function chunk(overrides: Partial<GraphNode> = {}): GraphNode {
  return { kind: 'chunk', id: 'c', label: 'C', ...overrides };
}
function contact(overrides: Partial<GraphNode> = {}): GraphNode {
  return { kind: 'contact', id: 'p', label: 'P', entity_kind: 'person', ...overrides };
}

describe('memoryGraphLayout', () => {
  it('colours summaries by level, wrapping the palette', () => {
    expect(levelColor(0)).toBe(LEVEL_COLOR[0]);
    expect(levelColor(2)).toBe(LEVEL_COLOR[2]);
    expect(levelColor(LEVEL_COLOR.length)).toBe(LEVEL_COLOR[0]); // wraps
    expect(levelColor(null)).toBe(LEAF_COLOR);
    expect(levelColor(-5)).toBe(LEVEL_COLOR[0]); // clamped to 0
  });

  it('nodeColor branches on kind', () => {
    expect(nodeColor(summary({ level: 1 }))).toBe(LEVEL_COLOR[1]);
    expect(nodeColor(chunk())).toBe(LEAF_COLOR);
    expect(nodeColor(contact())).toBe(CONTACT_COLOR);
  });

  it('nodeRadius grows with level and is fixed for chunk/contact', () => {
    expect(nodeRadius(summary({ level: 0 }))).toBe(5);
    expect(nodeRadius(summary({ level: 3 }))).toBe(12.5);
    expect(nodeRadius(summary({ level: 99 }))).toBe(252.5);
    expect(nodeRadius(contact())).toBe(9);
    expect(nodeRadius(chunk())).toBe(3);
  });

  it('only summary/contact nodes glow', () => {
    expect(nodeGlows(summary())).toBe(true);
    expect(nodeGlows(contact())).toBe(true);
    expect(nodeGlows(chunk())).toBe(false);
  });

  it('buildGraph derives parent_id edges in tree mode and drops danglers', () => {
    const nodes = [
      summary({ id: 'root', parent_id: null }),
      summary({ id: 'child', level: 1, parent_id: 'root' }),
      chunk({ id: 'leaf', parent_id: 'child' }),
      chunk({ id: 'orphan', parent_id: 'missing' }), // dangling → dropped
    ];
    const { simNodes, links } = buildGraph(nodes, [], 'tree');
    expect(simNodes).toHaveLength(4);
    expect(simNodes.every(n => typeof n.x === 'number' && typeof n.y === 'number')).toBe(true);
    const pairs = links.map(l => `${String(l.source)}->${String(l.target)}`);
    expect(pairs).toContain('child->root');
    expect(pairs).toContain('leaf->child');
    expect(pairs).not.toContain('orphan->missing');
  });

  it('buildGraph uses explicit edges in contacts mode and drops danglers', () => {
    const nodes = [chunk({ id: 'c1' }), contact({ id: 'p1' })];
    const edges: GraphEdge[] = [
      { from: 'c1', to: 'p1' },
      { from: 'c1', to: 'ghost' }, // dangling endpoint → dropped
    ];
    const { links } = buildGraph(nodes, edges, 'contacts');
    expect(links).toHaveLength(1);
    expect(String(links[0].source)).toBe('c1');
    expect(String(links[0].target)).toBe('p1');
  });

  it('createSimulation resolves link ids to node objects and converges', () => {
    const nodes = [summary({ id: 'root' }), summary({ id: 'child', parent_id: 'root' })];
    const { simNodes, links } = buildGraph(nodes, [], 'tree');
    const sim = createSimulation(simNodes, links);
    for (let i = 0; i < 50; i++) sim.tick();
    // forceLink replaces string ids with the actual node objects.
    expect((links[0].source as SimNode).id).toBe('child');
    expect((links[0].target as SimNode).id).toBe('root');
    expect(Number.isFinite(simNodes[0].x)).toBe(true);
    sim.stop();
  });

  it('pickNode returns the nearest node within its disc, else null', () => {
    const a: SimNode = { ...summary({ id: 'a', level: 0 }), x: 0, y: 0 };
    const b: SimNode = { ...summary({ id: 'b', level: 0 }), x: 100, y: 0 };
    expect(pickNode([a, b], 1, 1)?.id).toBe('a');
    expect(pickNode([a, b], 99, 0)?.id).toBe('b');
    expect(pickNode([a, b], 50, 50)).toBeNull(); // outside both discs
  });

  it('supportsWebGL is false under jsdom (no GL context)', () => {
    expect(supportsWebGL()).toBe(false);
  });
});
