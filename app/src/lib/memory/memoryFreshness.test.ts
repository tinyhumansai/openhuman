import { describe, expect, it } from 'vitest';

import { DAY, NOW, rel } from '../../test/memoryRelationFactory';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import { classify, computeFreshness, recallProbability, strengthFactor } from './memoryFreshness';

describe('strengthFactor', () => {
  it('scales the half-life by evidence with diminishing returns (log2)', () => {
    expect(strengthFactor(1)).toBeCloseTo(1, 12); // 1 + log2(1)
    expect(strengthFactor(2)).toBeCloseTo(2, 12); // 1 + log2(2)
    expect(strengthFactor(4)).toBeCloseTo(3, 12); // 1 + log2(4)
    expect(strengthFactor(8)).toBeCloseTo(4, 12);
  });

  it('clamps evidence <= 1 / non-finite to a factor of 1', () => {
    expect(strengthFactor(0)).toBe(1);
    expect(strengthFactor(-3)).toBe(1);
    expect(strengthFactor(Number.NaN)).toBe(1);
  });
});

describe('recallProbability', () => {
  it('is 1 at age 0 and halves every half-life', () => {
    expect(recallProbability(0, 30)).toBe(1);
    expect(recallProbability(30, 30)).toBeCloseTo(0.5, 12);
    expect(recallProbability(60, 30)).toBeCloseTo(0.25, 12);
    expect(recallProbability(90, 30)).toBeCloseTo(0.125, 12);
  });

  it('treats negative age as fresh (recall 1)', () => {
    expect(recallProbability(-5, 30)).toBe(1);
  });

  it('degrades gracefully for a non-positive half-life', () => {
    expect(recallProbability(0, 0)).toBe(1);
    expect(recallProbability(5, 0)).toBe(0);
  });
});

describe('classify', () => {
  it('bands recall into fresh / fading / stale', () => {
    expect(classify(0.95)).toBe('fresh');
    expect(classify(0.7)).toBe('fresh'); // boundary inclusive
    expect(classify(0.5)).toBe('fading');
    expect(classify(0.3)).toBe('fading'); // boundary inclusive
    expect(classify(0.2)).toBe('stale');
  });
});

describe('computeFreshness', () => {
  it('returns an empty/zero report for no relations', () => {
    const r = computeFreshness([], NOW);
    expect(r.facts).toEqual([]);
    expect(r.staleQueue).toEqual([]);
    expect(r.total).toBe(0);
    expect(r.averageRecall).toBe(0);
    expect(r.freshCount + r.fadingCount + r.staleCount).toBe(0);
  });

  it('scores a fresh / fading / stale mix and orders most-stale-first', () => {
    const r = computeFreshness(
      [
        rel('You', 'Berlin', 0), // recall 1 -> fresh
        rel('You', 'coffee', 30), // recall 0.5 -> fading
        rel('You', 'guitar', 90), // recall 0.125 -> stale
      ],
      NOW
    );
    expect(r.total).toBe(3);
    expect(r.freshCount).toBe(1);
    expect(r.fadingCount).toBe(1);
    expect(r.staleCount).toBe(1);
    expect(r.averageRecall).toBeCloseTo((1 + 0.5 + 0.125) / 3, 9);
    // most stale first
    expect(r.facts.map(f => f.object)).toEqual(['guitar', 'coffee', 'Berlin']);
    expect(r.facts[0].recall).toBeCloseTo(0.125, 9);
    // staleQueue excludes the fresh fact
    expect(r.staleQueue.map(f => f.object)).toEqual(['guitar', 'coffee']);
  });

  it('decays more evidence-rich facts more slowly', () => {
    const r = computeFreshness(
      [
        rel('A', 'weak', 60, 1), // halfLife 30 -> recall 0.25 (stale)
        rel('A', 'strong', 60, 2), // halfLife 60 -> recall 0.5 (fading)
      ],
      NOW
    );
    const byObject = Object.fromEntries(r.facts.map(f => [f.object, f]));
    expect(byObject.weak.recall).toBeCloseTo(0.25, 9);
    expect(byObject.strong.recall).toBeCloseTo(0.5, 9);
    expect(byObject.strong.recall).toBeGreaterThan(byObject.weak.recall);
  });

  it('treats a future updatedAt as fully fresh (no negative age)', () => {
    const future: GraphRelation = { ...rel('A', 'B', 0), updatedAt: NOW + 10 * DAY };
    const r = computeFreshness([future], NOW);
    expect(r.facts[0].ageDays).toBe(0);
    expect(r.facts[0].recall).toBe(1);
    expect(r.facts[0].status).toBe('fresh');
  });

  it('collapses a duplicate triple to its freshest occurrence', () => {
    const r = computeFreshness(
      [
        rel('You', 'Berlin', 90), // stale copy
        rel('You', 'Berlin', 0), // fresh copy (same triple)
      ],
      NOW
    );
    expect(r.total).toBe(1);
    expect(r.facts[0].recall).toBe(1); // kept the freshest
  });

  it('drops a malformed relation with a non-string endpoint', () => {
    const malformed = { ...rel('A', 'B', 0), predicate: null as unknown as string };
    const r = computeFreshness([rel('A', 'B', 0), malformed, rel('C', 'D', 0)], NOW);
    expect(r.total).toBe(2);
  });

  it('exposes the evidence-scaled half-life on each fact', () => {
    const r = computeFreshness([rel('A', 'B', 0, 4)], NOW);
    expect(r.facts[0].halfLifeDays).toBeCloseTo(90, 9); // 30 * (1 + log2(4))
  });
});
