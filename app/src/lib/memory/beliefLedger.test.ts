import { describe, expect, it } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands';
import {
  buildBeliefHistories,
  classifyRevision,
  computeConfidence,
  detectConflicts,
  groupRelationsByClaim,
  isMutuallyExclusive,
  MUTUALLY_EXCLUSIVE_PREDICATES,
  normalizeClaimKey,
  normalizeObject,
  normalizeText,
  type RelationVersion,
  resolveCurrentBelief,
  scoreVersion,
} from './beliefLedger';

// Fixed reference time so every scoring test is deterministic.
const NOW = 1_700_000_000;
const DAY = 86_400;

function makeRel(overrides: Partial<GraphRelation> = {}): GraphRelation {
  return {
    namespace: 'work',
    subject: 'You',
    predicate: 'lives_in',
    object: 'Berlin',
    attrs: {},
    updatedAt: NOW,
    evidenceCount: 1,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
    ...overrides,
  };
}

function makeVersion(overrides: Partial<RelationVersion> = {}): RelationVersion {
  return {
    object: 'Berlin',
    normalizedObject: 'berlin',
    updatedAt: NOW,
    evidenceCount: 1,
    namespace: 'work',
    documentIds: [],
    chunkIds: [],
    subjectType: null,
    objectType: null,
    ...overrides,
  };
}

describe('normalizeText / normalizeObject / normalizeClaimKey', () => {
  it('lowercases, maps underscores/dashes to spaces, collapses whitespace, trims', () => {
    expect(normalizeText('  Works_At ')).toBe('works at');
    expect(normalizeText('Reports-To')).toBe('reports to');
    expect(normalizeText('lives   in')).toBe('lives in');
  });

  it('normalizeObject strips trailing sentence punctuation', () => {
    expect(normalizeObject('Berlin.')).toBe('berlin');
    expect(normalizeObject('Lisbon!!!')).toBe('lisbon');
    expect(normalizeObject('a developer,')).toBe('a developer');
  });

  it('normalizeClaimKey merges case/underscore variants of subject+predicate', () => {
    expect(normalizeClaimKey('You', 'works_at')).toBe(normalizeClaimKey('you', 'Works At'));
    expect(normalizeClaimKey('You', 'lives in')).not.toBe(normalizeClaimKey('Bob', 'lives in'));
  });
});

describe('isMutuallyExclusive', () => {
  it('recognizes single-valued predicates regardless of formatting', () => {
    expect(isMutuallyExclusive('lives_in')).toBe(true);
    expect(isMutuallyExclusive('Works At')).toBe(true);
    expect(isMutuallyExclusive('prefers')).toBe(true);
  });

  it('returns false for additive predicates', () => {
    expect(isMutuallyExclusive('knows')).toBe(false);
    expect(isMutuallyExclusive('skilled_in')).toBe(false);
  });

  it('exposes the predicate set in normalized form', () => {
    expect(MUTUALLY_EXCLUSIVE_PREDICATES.has('lives in')).toBe(true);
    expect(MUTUALLY_EXCLUSIVE_PREDICATES.has('lives_in')).toBe(false);
  });
});

describe('groupRelationsByClaim', () => {
  it('buckets by normalized (subject, predicate)', () => {
    const groups = groupRelationsByClaim([
      makeRel({ subject: 'You', predicate: 'works_at', object: 'Acme' }),
      makeRel({ subject: 'you', predicate: 'Works At', object: 'Globex' }),
      makeRel({ subject: 'Bob', predicate: 'works_at', object: 'Initech' }),
    ]);
    expect(groups.size).toBe(2);
    expect(groups.get(normalizeClaimKey('You', 'works_at'))).toHaveLength(2);
  });

  it('skips relations with empty subject / predicate / object', () => {
    const groups = groupRelationsByClaim([
      makeRel({ subject: '   ', object: 'X' }),
      makeRel({ predicate: '', object: 'Y' }),
      makeRel({ object: '   ' }),
      makeRel({ subject: 'You', predicate: 'lives_in', object: 'Berlin' }),
    ]);
    expect(groups.size).toBe(1);
  });

  it('skips objects that are empty AFTER normalization (punctuation-only)', () => {
    const groups = groupRelationsByClaim([
      makeRel({ object: '...' }),
      makeRel({ object: '?!' }),
      makeRel({ object: 'Berlin' }),
    ]);
    const versions = [...groups.values()][0];
    expect(versions).toHaveLength(1);
    expect(versions[0].normalizedObject).toBe('berlin');
  });

  it('clamps negative / NaN evidence to 0 and reads entity_types from attrs', () => {
    const groups = groupRelationsByClaim([
      makeRel({
        evidenceCount: -5,
        attrs: { entity_types: { subject: 'Person', object: 'City' } },
      }),
    ]);
    const v = [...groups.values()][0][0];
    expect(v.evidenceCount).toBe(0);
    expect(v.subjectType).toBe('Person');
    expect(v.objectType).toBe('City');
  });
});

describe('detectConflicts', () => {
  it('reports a single distinct value as no conflict', () => {
    const r = detectConflicts([makeVersion(), makeVersion({ object: 'Berlin.' })]);
    expect(r.hasConflict).toBe(false);
    expect(r.distinctValueCount).toBe(1);
  });

  it('reports two distinct normalized values as a conflict', () => {
    const r = detectConflicts([
      makeVersion({ normalizedObject: 'berlin' }),
      makeVersion({ normalizedObject: 'lisbon' }),
    ]);
    expect(r.hasConflict).toBe(true);
    expect(r.distinctValueCount).toBe(2);
  });
});

describe('scoreVersion', () => {
  it('decays to ~1 at now and ~0.5 after one half-life (equal evidence)', () => {
    const fresh = scoreVersion(makeVersion({ updatedAt: NOW, evidenceCount: 0 }), NOW);
    const oneHalfLife = scoreVersion(
      makeVersion({ updatedAt: NOW - 180 * DAY, evidenceCount: 0 }),
      NOW
    );
    expect(fresh).toBeCloseTo(1, 5);
    expect(oneHalfLife).toBeCloseTo(0.5, 5);
  });

  it('rewards higher evidence at equal time', () => {
    const lo = scoreVersion(makeVersion({ evidenceCount: 1 }), NOW);
    const hi = scoreVersion(makeVersion({ evidenceCount: 50 }), NOW);
    expect(hi).toBeGreaterThan(lo);
  });

  it('lets recency outweigh a stale-but-high-evidence value past the half-life', () => {
    const stale = scoreVersion(makeVersion({ updatedAt: NOW - 400 * DAY, evidenceCount: 20 }), NOW);
    const fresh = scoreVersion(makeVersion({ updatedAt: NOW, evidenceCount: 1 }), NOW);
    expect(fresh).toBeGreaterThan(stale);
  });
});

describe('resolveCurrentBelief', () => {
  it('throws on empty input', () => {
    expect(() => resolveCurrentBelief([], NOW)).toThrow();
  });

  it('picks the newer value when evidence is equal (recency wins)', () => {
    const { current, runnerUp } = resolveCurrentBelief(
      [
        makeVersion({ normalizedObject: 'berlin', object: 'Berlin', updatedAt: NOW - 300 * DAY }),
        makeVersion({ normalizedObject: 'lisbon', object: 'Lisbon', updatedAt: NOW }),
      ],
      NOW
    );
    expect(current.normalizedObject).toBe('lisbon');
    expect(runnerUp?.normalizedObject).toBe('berlin');
  });

  it('runnerUp is the best of the OTHER distinct values, not a duplicate of the winner', () => {
    const { current, runnerUp } = resolveCurrentBelief(
      [
        makeVersion({ normalizedObject: 'lisbon', updatedAt: NOW }),
        makeVersion({ normalizedObject: 'lisbon', updatedAt: NOW - DAY }),
        makeVersion({ normalizedObject: 'berlin', updatedAt: NOW - 10 * DAY }),
      ],
      NOW
    );
    expect(current.normalizedObject).toBe('lisbon');
    expect(runnerUp?.normalizedObject).toBe('berlin');
  });

  it('returns null runnerUp when only one distinct value exists', () => {
    const { runnerUp } = resolveCurrentBelief(
      [makeVersion({ object: 'Berlin' }), makeVersion({ object: 'Berlin.' })],
      NOW
    );
    expect(runnerUp).toBeNull();
  });

  it('breaks exact ties deterministically (updatedAt, then evidence, then lexicographic)', () => {
    const a = makeVersion({ normalizedObject: 'alpha', updatedAt: NOW, evidenceCount: 3 });
    const b = makeVersion({ normalizedObject: 'beta', updatedAt: NOW, evidenceCount: 3 });
    const r1 = resolveCurrentBelief([a, b], NOW);
    const r2 = resolveCurrentBelief([b, a], NOW);
    // Fully tied on score/time/evidence -> lexicographic 'alpha' wins, order-independent.
    expect(r1.current.normalizedObject).toBe('alpha');
    expect(r2.current.normalizedObject).toBe('alpha');
  });
});

describe('computeConfidence', () => {
  it('is 1 when there is no runner-up', () => {
    expect(computeConfidence(makeVersion(), null, NOW)).toBe(1);
  });

  it('is near 0 for a dead heat and high for a lopsided margin', () => {
    const tie = computeConfidence(
      makeVersion({ updatedAt: NOW, evidenceCount: 2 }),
      makeVersion({ updatedAt: NOW, evidenceCount: 2 }),
      NOW
    );
    const lopsided = computeConfidence(
      makeVersion({ updatedAt: NOW, evidenceCount: 5 }),
      makeVersion({ updatedAt: NOW - 500 * DAY, evidenceCount: 0 }),
      NOW
    );
    expect(tie).toBeCloseTo(0, 5);
    expect(lopsided).toBeGreaterThan(0.8);
    expect(lopsided).toBeLessThanOrEqual(1);
  });

  it('returns 0 (no divide-by-zero/NaN) when both scores underflow at extreme age', () => {
    const ancient = makeVersion({ updatedAt: NOW - 600 * 365 * DAY, evidenceCount: 0 });
    const ancient2 = makeVersion({
      normalizedObject: 'other',
      updatedAt: NOW - 600 * 365 * DAY,
      evidenceCount: 0,
    });
    expect(computeConfidence(ancient, ancient2, NOW)).toBe(0);
  });

  it('stays within [0,1] and is rounded to 4 decimals', () => {
    const c = computeConfidence(
      makeVersion({ evidenceCount: 9 }),
      makeVersion({ evidenceCount: 1 }),
      NOW
    );
    expect(c).toBeGreaterThanOrEqual(0);
    expect(c).toBeLessThanOrEqual(1);
    expect(Number(c.toFixed(4))).toBe(c);
  });
});

describe('classifyRevision', () => {
  const current = makeVersion({ normalizedObject: 'lisbon' });

  it('marks a version equal to the current belief as reinforced', () => {
    expect(classifyRevision(makeVersion({ normalizedObject: 'lisbon' }), current, null, true)).toBe(
      'reinforced'
    );
  });

  it('marks the first (no predecessor) differing version as refined', () => {
    expect(classifyRevision(makeVersion({ normalizedObject: 'berlin' }), current, null, true)).toBe(
      'refined'
    );
  });

  it('marks a value change on a mutually-exclusive predicate as contradicted', () => {
    const prev = makeVersion({ normalizedObject: 'berlin' });
    const ver = makeVersion({ normalizedObject: 'munich' });
    expect(classifyRevision(ver, current, prev, true)).toBe('contradicted');
  });

  it('marks a substring extension as refined even on a non-exclusive predicate', () => {
    const prev = makeVersion({ normalizedObject: 'rust' });
    const ver = makeVersion({ normalizedObject: 'rust and go' });
    expect(classifyRevision(ver, current, prev, false)).toBe('refined');
  });

  it('marks an unrelated value change on a non-exclusive predicate as superseded', () => {
    const prev = makeVersion({ normalizedObject: 'rust' });
    const ver = makeVersion({ normalizedObject: 'python' });
    expect(classifyRevision(ver, current, prev, false)).toBe('superseded');
  });

  it('does NOT call incidental substring overlap a refinement (token-subset only)', () => {
    // "go" is a substring of "mango" but not a token of it -> superseded, not refined.
    const prev = makeVersion({ normalizedObject: 'go' });
    const ver = makeVersion({ normalizedObject: 'mango' });
    expect(classifyRevision(ver, current, prev, false)).toBe('superseded');
  });

  it('treats a value identical to its predecessor as reinforced', () => {
    const prev = makeVersion({ normalizedObject: 'berlin' });
    const ver = makeVersion({ normalizedObject: 'berlin' });
    expect(classifyRevision(ver, current, prev, true)).toBe('reinforced');
  });
});

describe('buildBeliefHistories', () => {
  it('returns [] for empty input', () => {
    expect(buildBeliefHistories([], NOW)).toEqual([]);
  });

  it('builds a single high-confidence history from one relation', () => {
    const out = buildBeliefHistories([makeRel({ object: 'Berlin' })], NOW);
    expect(out).toHaveLength(1);
    expect(out[0].hasConflict).toBe(false);
    expect(out[0].confidence).toBe(1);
    expect(out[0].current.normalizedObject).toBe('berlin');
    expect(out[0].revisions).toHaveLength(1);
    expect(out[0].revisions[0].isCurrent).toBe(true);
  });

  it('treats a reinforcement chain (same value x3) as no conflict, all reinforced', () => {
    const out = buildBeliefHistories(
      [
        makeRel({ object: 'Berlin', updatedAt: NOW - 20 * DAY, evidenceCount: 1 }),
        makeRel({ object: 'Berlin.', updatedAt: NOW - 10 * DAY, evidenceCount: 2 }),
        makeRel({ object: 'berlin', updatedAt: NOW, evidenceCount: 3 }),
      ],
      NOW
    );
    expect(out).toHaveLength(1);
    expect(out[0].hasConflict).toBe(false);
    expect(out[0].distinctValueCount).toBe(1);
    expect(out[0].revisions.every(r => r.kind === 'reinforced')).toBe(true);
  });

  it('detects a hard contradiction: lives_in Berlin then Lisbon -> current Lisbon, contradicted', () => {
    const out = buildBeliefHistories(
      [
        makeRel({ predicate: 'lives_in', object: 'Berlin', updatedAt: NOW - 120 * DAY }),
        makeRel({ predicate: 'lives_in', object: 'Lisbon', updatedAt: NOW }),
      ],
      NOW
    );
    expect(out).toHaveLength(1);
    const h = out[0];
    expect(h.hasConflict).toBe(true);
    expect(h.mutuallyExclusive).toBe(true);
    expect(h.current.normalizedObject).toBe('lisbon');
    // newest revision first; the newer Lisbon differs from prior Berlin on an
    // exclusive predicate, but it equals the current belief -> reinforced;
    // the older Berlin (first, no predecessor) -> refined.
    expect(h.revisions[0].version.normalizedObject).toBe('lisbon');
    expect(h.revisions[0].isCurrent).toBe(true);
    const berlin = h.revisions.find(r => r.version.normalizedObject === 'berlin');
    expect(berlin?.kind).toBe('refined');
  });

  it('flags contradiction on the middle value of a three-step exclusive chain', () => {
    const out = buildBeliefHistories(
      [
        makeRel({ predicate: 'lives_in', object: 'Berlin', updatedAt: NOW - 200 * DAY }),
        makeRel({ predicate: 'lives_in', object: 'Munich', updatedAt: NOW - 100 * DAY }),
        makeRel({ predicate: 'lives_in', object: 'Lisbon', updatedAt: NOW }),
      ],
      NOW
    );
    const h = out[0];
    expect(h.current.normalizedObject).toBe('lisbon');
    // Munich changed from Berlin on an exclusive predicate -> contradicted.
    const munich = h.revisions.find(r => r.version.normalizedObject === 'munich');
    expect(munich?.kind).toBe('contradicted');
  });

  it('orders output most-contested first (conflict before non-conflict)', () => {
    const out = buildBeliefHistories(
      [
        // Clean claim (no conflict).
        makeRel({ subject: 'You', predicate: 'skilled_in', object: 'Rust' }),
        // Contested claim (two values).
        makeRel({
          subject: 'You',
          predicate: 'lives_in',
          object: 'Berlin',
          updatedAt: NOW - 90 * DAY,
        }),
        makeRel({ subject: 'You', predicate: 'lives_in', object: 'Lisbon', updatedAt: NOW }),
      ],
      NOW
    );
    expect(out).toHaveLength(2);
    expect(out[0].hasConflict).toBe(true);
    expect(out[1].hasConflict).toBe(false);
  });

  it('keeps display subject/predicate as the first-seen raw form', () => {
    const out = buildBeliefHistories(
      [makeRel({ subject: 'You', predicate: 'works_at', object: 'Acme' })],
      NOW
    );
    expect(out[0].subject).toBe('You');
    expect(out[0].predicate).toBe('works_at');
  });

  it('sorts revisions newest -> oldest', () => {
    const out = buildBeliefHistories(
      [
        makeRel({ object: 'A', updatedAt: NOW - 50 * DAY }),
        makeRel({ object: 'B', updatedAt: NOW - 10 * DAY }),
        makeRel({ object: 'C', updatedAt: NOW }),
      ],
      NOW
    );
    const times = out[0].revisions.map(r => r.version.updatedAt);
    expect(times).toEqual([...times].sort((a, b) => b - a));
  });
});
