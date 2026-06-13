import { describe, expect, it } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { findConnectionPath } from './connectionPath';

function rel(subject: string, object: string, predicate = 'p'): GraphRelation {
  return {
    namespace: 'n',
    subject,
    predicate,
    object,
    attrs: {},
    updatedAt: 0,
    evidenceCount: 1,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
  };
}

describe('findConnectionPath', () => {
  it('finds a simple forward chain A -> B -> C', () => {
    const r = findConnectionPath([rel('A', 'B', 'knows'), rel('B', 'C', 'likes')], 'A', 'C');
    expect(r.found).toBe(true);
    expect(r.reason).toBe('ok');
    expect(r.length).toBe(2);
    expect(r.hops).toEqual([
      { from: 'A', to: 'B', predicate: 'knows', forward: true },
      { from: 'B', to: 'C', predicate: 'likes', forward: true },
    ]);
  });

  it('traverses edges regardless of direction and records forward=false for reversed hops', () => {
    // A->B and C->B: A connects to C through B, the B->C hop is reversed.
    const r = findConnectionPath([rel('A', 'B'), rel('C', 'B')], 'A', 'C');
    expect(r.found).toBe(true);
    expect(r.length).toBe(2);
    expect(r.hops[0]).toEqual({ from: 'A', to: 'B', predicate: 'p', forward: true });
    expect(r.hops[1]).toEqual({ from: 'B', to: 'C', predicate: 'p', forward: false });
  });

  it('returns same for identical present endpoints', () => {
    const r = findConnectionPath([rel('A', 'B')], 'A', 'A');
    expect(r.found).toBe(true);
    expect(r.reason).toBe('same');
    expect(r.length).toBe(0);
    expect(r.hops).toEqual([]);
  });

  it('reports same before missing when identical endpoints are absent from the graph', () => {
    // Two identical (even nonexistent) inputs should prompt "pick two different",
    // not a misleading missing-node message.
    const r = findConnectionPath([rel('A', 'B')], 'Z', 'Z');
    expect(r.reason).toBe('same');
    expect(r.found).toBe(true);
    expect(r.length).toBe(0);
    expect(r.hops).toEqual([]);
  });

  it('returns same with no BFS even on an empty graph', () => {
    const r = findConnectionPath([], 'A', 'A');
    expect(r.reason).toBe('same');
    expect(r.found).toBe(true);
    expect(r.hops).toEqual([]);
  });

  it('flags a missing source or target', () => {
    expect(findConnectionPath([rel('A', 'B')], 'Z', 'B').reason).toBe('missing-source');
    expect(findConnectionPath([rel('A', 'B')], 'A', 'Z').reason).toBe('missing-target');
  });

  it('returns missing-source/target on an empty graph for distinct endpoints', () => {
    expect(findConnectionPath([], 'A', 'B').reason).toBe('missing-source');
  });

  it('reports no-path across disconnected components', () => {
    const r = findConnectionPath([rel('A', 'B'), rel('C', 'D')], 'A', 'D');
    expect(r.found).toBe(false);
    expect(r.reason).toBe('no-path');
    expect(r.hops).toEqual([]);
  });

  it('returns a shortest path and breaks ties deterministically (smallest neighbour first)', () => {
    // A reaches D via B or via C (both length 2); B sorts before C, so via B.
    const triples = [rel('A', 'B'), rel('A', 'C'), rel('B', 'D'), rel('C', 'D')];
    const r = findConnectionPath(triples, 'A', 'D');
    expect(r.length).toBe(2);
    expect(r.hops.map(h => h.to)).toEqual(['B', 'D']);
    // Order-invariant: shuffling the relations yields the identical path.
    const shuffled = findConnectionPath([...triples].reverse(), 'A', 'D');
    expect(shuffled.hops).toEqual(r.hops);
  });

  it('ignores self-loops when pathing', () => {
    // A's self-loop must not appear as a hop; A->B->C is the real path.
    const r = findConnectionPath([rel('A', 'A'), rel('A', 'B'), rel('B', 'C')], 'A', 'C');
    expect(r.length).toBe(2);
    expect(r.hops.every(h => h.from !== h.to)).toBe(true);
  });

  it('takes the direct one-hop path when entities are directly linked', () => {
    const r = findConnectionPath([rel('A', 'B'), rel('B', 'C'), rel('A', 'C')], 'A', 'C');
    expect(r.length).toBe(1);
    expect(r.hops[0]).toMatchObject({ from: 'A', to: 'C' });
  });

  it('drops malformed relations with a non-string endpoint', () => {
    const malformed = { ...rel('B', 'C'), subject: null as unknown as string };
    // The B->C edge is malformed/dropped, so A and C are disconnected.
    const r = findConnectionPath([rel('A', 'B'), malformed], 'A', 'C');
    expect(r.reason).toBe('missing-target'); // C never entered the graph
  });
});
