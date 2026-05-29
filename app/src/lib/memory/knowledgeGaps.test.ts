import { describe, expect, it } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { computeKnowledgeGaps } from './knowledgeGaps';

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

describe('computeKnowledgeGaps', () => {
  it('returns an empty report for no relations', () => {
    expect(computeKnowledgeGaps([])).toEqual({
      gaps: [],
      orphanCount: 0,
      leafCount: 0,
      connectedCount: 0,
      entityCount: 0,
      gapRatio: 0,
    });
  });

  it('classifies orphans, leaves, and connected entities', () => {
    // A->B->C chain + a self-loop orphan D.
    const r = computeKnowledgeGaps([rel('A', 'B'), rel('B', 'C'), rel('D', 'D')]);
    expect(r.entityCount).toBe(4);
    expect(r.orphanCount).toBe(1); // D (self-loop only)
    expect(r.leafCount).toBe(2); // A and C
    expect(r.connectedCount).toBe(1); // B (degree 2)
    expect(r.gapRatio).toBeCloseTo(0.75, 12);
    // Orphan first, then leaves by id asc.
    expect(r.gaps.map(g => g.id)).toEqual(['D', 'A', 'C']);
    expect(r.gaps[0]).toEqual({ id: 'D', degree: 0, kind: 'orphan', objectOnly: false });
  });

  it('flags entities that appear only as an object (mentioned but never described)', () => {
    const r = computeKnowledgeGaps([rel('X', 'Y')]);
    const byId = Object.fromEntries(r.gaps.map(g => [g.id, g]));
    expect(byId.X.objectOnly).toBe(false); // X is a subject
    expect(byId.Y.objectOnly).toBe(true); // Y only ever an object
    expect(byId.X.kind).toBe('leaf');
    expect(byId.Y.kind).toBe('leaf');
  });

  it('treats a self-loop-only entity as an orphan (degree 0)', () => {
    const r = computeKnowledgeGaps([rel('A', 'A')]);
    expect(r.entityCount).toBe(1);
    expect(r.gaps).toEqual([{ id: 'A', degree: 0, kind: 'orphan', objectOnly: false }]);
  });

  it('excludes well-connected entities from the gap list', () => {
    const r = computeKnowledgeGaps([rel('A', 'B'), rel('B', 'C'), rel('C', 'A')]);
    expect(r.gaps).toEqual([]);
    expect(r.connectedCount).toBe(3);
    expect(r.gapRatio).toBe(0);
  });

  it('is invariant to relation order', () => {
    const triples = [rel('A', 'B'), rel('B', 'C'), rel('D', 'D')];
    const forward = computeKnowledgeGaps(triples);
    const reversed = computeKnowledgeGaps([...triples].reverse());
    expect(reversed).toEqual(forward);
  });

  it('drops malformed relations with a non-string endpoint', () => {
    const malformed = { ...rel('A', 'B'), object: null as unknown as string };
    const r = computeKnowledgeGaps([rel('A', 'B'), malformed, rel('C', 'D')]);
    // A,B,C,D only; the null-object row is ignored.
    expect(r.entityCount).toBe(4);
  });
});
