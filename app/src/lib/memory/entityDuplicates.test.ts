import { describe, expect, it } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { computeEntityDuplicates, normalizeEntity } from './entityDuplicates';

function rel(subject: string, object: string): GraphRelation {
  return {
    namespace: 'n',
    subject,
    predicate: 'p',
    object,
    attrs: {},
    updatedAt: 0,
    evidenceCount: 1,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
  };
}

describe('normalizeEntity', () => {
  it('trims, collapses internal whitespace, and lowercases', () => {
    expect(normalizeEntity('  Alice ')).toBe('alice');
    expect(normalizeEntity('New  York')).toBe('new york');
    expect(normalizeEntity('ALICE')).toBe('alice');
  });
});

describe('computeEntityDuplicates', () => {
  it('returns an empty report for no relations', () => {
    expect(computeEntityDuplicates([])).toEqual({
      clusters: [],
      clusterCount: 0,
      affectedEntities: 0,
      entityCount: 0,
    });
  });

  it('groups case/whitespace variants into one cluster', () => {
    const r = computeEntityDuplicates([
      rel('Alice', 'Bob'),
      rel('alice', 'Carol'),
      rel(' Alice ', 'Dave'),
    ]);
    expect(r.entityCount).toBe(6); // 3 Alice-variants + Bob, Carol, Dave
    expect(r.clusterCount).toBe(1);
    expect(r.clusters[0].normalized).toBe('alice');
    expect(r.clusters[0].variants.map(v => v.id).sort()).toEqual([' Alice ', 'Alice', 'alice']);
    expect(r.affectedEntities).toBe(3);
  });

  it('does not flag entities with distinct normalized forms', () => {
    const r = computeEntityDuplicates([rel('Alice', 'Bob')]);
    expect(r.entityCount).toBe(2);
    expect(r.clusters).toEqual([]);
  });

  it('orders variants within a cluster by degree desc', () => {
    // "Alice" connects to B and C (degree 2); "alice" connects to D (degree 1).
    const r = computeEntityDuplicates([rel('Alice', 'B'), rel('Alice', 'C'), rel('alice', 'D')]);
    expect(r.clusters[0].variants[0]).toEqual({ id: 'Alice', degree: 2 });
    expect(r.clusters[0].variants[1]).toEqual({ id: 'alice', degree: 1 });
    expect(r.clusters[0].totalDegree).toBe(3);
  });

  it('ranks clusters by variant count desc', () => {
    const r = computeEntityDuplicates([
      // 3-variant cluster on "x"
      rel('X', 'a'),
      rel('x', 'b'),
      rel(' x ', 'c'),
      // 2-variant cluster on "y"
      rel('Y', 'd'),
      rel('y', 'e'),
    ]);
    expect(r.clusters.map(c => c.variants.length)).toEqual([3, 2]);
    expect(r.clusters[0].normalized).toBe('x');
  });

  it('keeps self-loop-only entities as degree-0 nodes that can still cluster', () => {
    const r = computeEntityDuplicates([rel('Alice', 'Alice'), rel('alice', 'alice')]);
    expect(r.entityCount).toBe(2);
    expect(r.clusterCount).toBe(1);
    expect(r.clusters[0].variants).toEqual([
      { id: 'Alice', degree: 0 },
      { id: 'alice', degree: 0 },
    ]);
    expect(r.clusters[0].totalDegree).toBe(0);
  });

  it('breaks cluster ties by totalDegree desc, then normalized key asc', () => {
    // Two 2-variant clusters; the "b" cluster is more connected (totalDegree 3 vs 2).
    const byDegree = computeEntityDuplicates([
      rel('A', 'x'),
      rel('a', 'y'),
      rel('B', 'p'),
      rel('B', 'q'),
      rel('b', 'r'),
    ]);
    expect(byDegree.clusters.map(c => c.normalized)).toEqual(['b', 'a']);

    // Two 2-variant clusters with EQUAL totalDegree -> fall back to key asc.
    const byKey = computeEntityDuplicates([
      rel('B', 'p'),
      rel('b', 'q'),
      rel('A', 'r'),
      rel('a', 's'),
    ]);
    expect(byKey.clusters.map(c => c.normalized)).toEqual(['a', 'b']);
  });

  it('is invariant to relation order', () => {
    const triples = [rel('Alice', 'Bob'), rel('alice', 'Carol'), rel(' Alice ', 'Dave')];
    const forward = computeEntityDuplicates(triples);
    const reversed = computeEntityDuplicates([...triples].reverse());
    expect(reversed).toEqual(forward);
  });

  it('drops malformed relations with a non-string endpoint', () => {
    const malformed = { ...rel('Alice', 'Bob'), object: null as unknown as string };
    const r = computeEntityDuplicates([rel('Alice', 'Bob'), malformed, rel('alice', 'Carol')]);
    // Alice/alice still cluster; the null-object row is ignored.
    expect(r.clusterCount).toBe(1);
    expect(r.clusters[0].variants).toHaveLength(2);
  });
});
