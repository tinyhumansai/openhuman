import { describe, expect, it } from 'vitest';

import { type GraphEdge, type GraphNode } from '../../utils/tauriCommands';
import { seedSvgLayout } from './seedSvgLayout';

const src = (id: string): GraphNode => ({ kind: 'source', id, label: id, parent_id: null });
const sum = (id: string, parent: string): GraphNode => ({
  kind: 'summary',
  id,
  label: id,
  level: 1,
  parent_id: parent,
});
const chunk = (id: string, parent: string): GraphNode => ({
  kind: 'chunk',
  id,
  label: id,
  parent_id: parent,
});
const contact = (id: string): GraphNode => ({
  kind: 'contact',
  id,
  label: id,
  entity_kind: 'person',
});

const NO_PREV = new Map<string, { x: number; y: number }>();
const finite = (p: { x: number; y: number }) => Number.isFinite(p.x) && Number.isFinite(p.y);

describe('seedSvgLayout — first load (no previous positions)', () => {
  it('marks every node new, reheats fully, and produces finite positions', () => {
    const nodes = [src('s'), sum('a', 's'), chunk('c1', 'a'), chunk('c2', 'a')];
    const r = seedSvgLayout(nodes, [], 'tree', NO_PREV);
    expect(r.positions).toHaveLength(4);
    expect(r.positions.every(p => p.isNew)).toBe(true);
    expect(r.newCount).toBe(4);
    expect(r.reheatAlpha).toBe(1);
    expect(r.positions.every(finite)).toBe(true);
  });

  it('handles an empty graph', () => {
    const r = seedSvgLayout([], [], 'tree', NO_PREV);
    expect(r.positions).toEqual([]);
    expect(r.edges).toEqual([]);
    expect(r.newCount).toBe(0);
    expect(r.reheatAlpha).toBe(1);
  });
});

describe('seedSvgLayout — carry-over', () => {
  it('preserves the exact previous position of surviving nodes', () => {
    const nodes = [src('s'), sum('a', 's')];
    const prev = new Map([
      ['s', { x: 100, y: 120 }],
      ['a', { x: 300, y: 340 }],
    ]);
    const r = seedSvgLayout(nodes, [], 'tree', prev);
    expect(r.positions[0]).toEqual({ x: 100, y: 120, isNew: false });
    expect(r.positions[1]).toEqual({ x: 300, y: 340, isNew: false });
    expect(r.newCount).toBe(0);
    expect(r.reheatAlpha).toBe(0.3); // all survived → gentle reheat
  });

  it('drops removed nodes — result length tracks the current node set', () => {
    const nodes = [src('s')];
    const prev = new Map([
      ['s', { x: 10, y: 10 }],
      ['gone-1', { x: 1, y: 1 }],
      ['gone-2', { x: 2, y: 2 }],
    ]);
    const r = seedSvgLayout(nodes, [], 'tree', prev);
    expect(r.positions).toHaveLength(1);
    expect(r.positions[0]).toEqual({ x: 10, y: 10, isNew: false });
  });
});

describe('seedSvgLayout — new nodes', () => {
  it('seeds a new child near its parent’s previous position', () => {
    const nodes = [sum('a', 's'), chunk('c', 'a')];
    const prev = new Map([['a', { x: 500, y: 250 }]]); // parent survived, child is new
    const r = seedSvgLayout(nodes, [], 'tree', prev);
    const childPos = r.positions[1];
    expect(childPos.isNew).toBe(true);
    const dist = Math.hypot(childPos.x - 500, childPos.y - 250);
    expect(dist).toBeGreaterThan(0);
    expect(dist).toBeLessThanOrEqual(25); // within the small offset of the parent
  });

  it('falls back to the ring when a new node’s parent is also new (not in prev)', () => {
    const nodes = [sum('a', 's'), chunk('c', 'a')]; // neither in prev
    const r = seedSvgLayout(nodes, [], 'tree', NO_PREV);
    expect(r.positions[1].isNew).toBe(true);
    expect(finite(r.positions[1])).toBe(true);
    // not clustered at the parent (parent has no known position)
    expect(r.positions[1]).not.toEqual(r.positions[0]);
  });

  it('falls back to the ring for a parent-less new node', () => {
    const nodes = [src('s')];
    const r = seedSvgLayout(nodes, [], 'tree', NO_PREV);
    expect(r.positions[0].isNew).toBe(true);
    expect(finite(r.positions[0])).toBe(true);
  });

  it('reheats fully when the graph is entirely replaced (no survivors)', () => {
    const nodes = [src('x'), sum('y', 'x')];
    const prev = new Map([['old', { x: 1, y: 2 }]]); // stale ids, none match
    const r = seedSvgLayout(nodes, [], 'tree', prev);
    expect(r.newCount).toBe(2);
    expect(r.reheatAlpha).toBe(1);
  });

  it('reheats gently when at least one node survives a mixed update', () => {
    const nodes = [sum('a', 's'), chunk('c1', 'a'), chunk('c2', 'a')];
    const prev = new Map([['a', { x: 400, y: 300 }]]); // a survives, c1/c2 new
    const r = seedSvgLayout(nodes, [], 'tree', prev);
    expect(r.newCount).toBe(2);
    expect(r.positions[0].isNew).toBe(false);
    expect(r.reheatAlpha).toBe(0.3);
  });
});

describe('seedSvgLayout — edges', () => {
  it('derives tree edges from parent_id', () => {
    const nodes = [src('s'), sum('a', 's'), chunk('c', 'a')];
    const r = seedSvgLayout(nodes, [], 'tree', NO_PREV);
    // [child, parent] index pairs: a→s (1,0), c→a (2,1)
    expect(r.edges).toEqual([
      [1, 0],
      [2, 1],
    ]);
  });

  it('skips tree edges whose parent_id is missing from the node set', () => {
    const nodes = [sum('a', 'ghost-parent'), chunk('c', 'a')];
    const r = seedSvgLayout(nodes, [], 'tree', NO_PREV);
    expect(r.edges).toEqual([[1, 0]]); // a→ghost dropped; c→a kept
    expect(r.edges.every(([x, y]) => x < nodes.length && y < nodes.length)).toBe(true);
  });

  it('uses explicit edges in contacts mode and skips dangling endpoints', () => {
    const nodes = [chunk('c', 'whatever'), contact('p')];
    const edges: GraphEdge[] = [
      { from: 'c', to: 'p' },
      { from: 'c', to: 'missing' }, // dangling → skipped
    ];
    const r = seedSvgLayout(nodes, edges, 'contacts', NO_PREV);
    expect(r.edges).toEqual([[0, 1]]);
    expect(r.edges.every(([x, y]) => x < nodes.length && y < nodes.length)).toBe(true);
  });

  it('seeds new contact-mode nodes on the ring (no parent_id seeding)', () => {
    const nodes = [chunk('c', 'x'), contact('p')];
    const r = seedSvgLayout(nodes, [{ from: 'c', to: 'p' }], 'contacts', NO_PREV);
    expect(r.positions.every(p => p.isNew && finite(p))).toBe(true);
  });
});

describe('seedSvgLayout — invariants', () => {
  it('is deterministic for identical inputs', () => {
    const nodes = [src('s'), sum('a', 's'), chunk('c1', 'a'), chunk('c2', 'a')];
    const a = seedSvgLayout(nodes, [], 'tree', NO_PREV);
    const b = seedSvgLayout(nodes, [], 'tree', NO_PREV);
    expect(a).toEqual(b);
  });

  it('keeps positions index-aligned with the node array', () => {
    const nodes = [src('s'), sum('a', 's'), chunk('c', 'a')];
    const r = seedSvgLayout(nodes, [], 'tree', new Map([['a', { x: 9, y: 9 }]]));
    expect(r.positions).toHaveLength(nodes.length);
  });
});
