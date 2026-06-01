import { describe, expect, it } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { computeNamespaceOverview } from './namespaceOverview';

function rel(namespace: string | null, subject: string, object: string): GraphRelation {
  return {
    namespace,
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

describe('computeNamespaceOverview', () => {
  it('returns an empty report for no relations', () => {
    expect(computeNamespaceOverview([])).toEqual({
      namespaces: [],
      namespaceCount: 0,
      totalFacts: 0,
      totalEntities: 0,
    });
  });

  it('aggregates distinct facts and entities per namespace', () => {
    const r = computeNamespaceOverview([
      rel('work', 'A', 'B'),
      rel('work', 'B', 'C'),
      rel('personal', 'X', 'Y'),
      rel(null, 'P', 'Q'),
    ]);
    expect(r.namespaceCount).toBe(3);
    expect(r.totalFacts).toBe(4);
    expect(r.totalEntities).toBe(7); // A,B,C,X,Y,P,Q
    // Sorted by factCount desc; ties by namespace asc with null last.
    expect(r.namespaces).toEqual([
      { namespace: 'work', factCount: 2, entityCount: 3 },
      { namespace: 'personal', factCount: 1, entityCount: 2 },
      { namespace: null, factCount: 1, entityCount: 2 },
    ]);
  });

  it('de-duplicates repeated triples within a namespace', () => {
    const r = computeNamespaceOverview([rel('work', 'A', 'B'), rel('work', 'A', 'B')]);
    expect(r.namespaces[0].factCount).toBe(1);
    expect(r.totalFacts).toBe(1);
  });

  it('counts a shared entity per-namespace but once globally', () => {
    const r = computeNamespaceOverview([rel('work', 'A', 'B'), rel('personal', 'A', 'C')]);
    const byNs = Object.fromEntries(r.namespaces.map(s => [s.namespace, s]));
    expect(byNs.work.entityCount).toBe(2); // A, B
    expect(byNs.personal.entityCount).toBe(2); // A, C
    expect(r.totalEntities).toBe(3); // A counted once globally
  });

  it('sorts the un-namespaced (null) bucket last on a tie', () => {
    const r = computeNamespaceOverview([rel(null, 'A', 'B'), rel('aaa', 'C', 'D')]);
    expect(r.namespaces.map(s => s.namespace)).toEqual(['aaa', null]);
  });

  it('drops malformed relations with a non-string field', () => {
    const malformed = { ...rel('work', 'A', 'B'), object: null as unknown as string };
    const r = computeNamespaceOverview([rel('work', 'A', 'B'), malformed, rel('work', 'C', 'D')]);
    expect(r.totalFacts).toBe(2);
  });

  it('is invariant to relation order', () => {
    const triples = [rel('work', 'A', 'B'), rel('personal', 'X', 'Y'), rel('work', 'B', 'C')];
    const forward = computeNamespaceOverview(triples);
    const reversed = computeNamespaceOverview([...triples].reverse());
    expect(reversed).toEqual(forward);
  });
});
