import { describe, expect, it } from 'vitest';

import {
  addRecord,
  baseRate,
  binIndex,
  brierScore,
  buildBins,
  buildCalibrationReport,
  buildTrend,
  deleteRecord,
  isOverdue,
  meanConfidence,
  murphyDecomposition,
  normalizeConfidence,
  outcomeToNumber,
  parsePredictionRecords,
  type PredictionOutcome,
  type PredictionRecord,
  resolveRecord,
} from './predictionLedger';

const NOW = 1_700_000_000;

let idSeq = 0;
function rec(overrides: Partial<PredictionRecord> = {}): PredictionRecord {
  idSeq += 1;
  return {
    id: `p${idSeq}`,
    statement: 'It will rain',
    confidence: 0.5,
    createdAt: NOW,
    resolveBy: null,
    outcome: null,
    resolvedAt: null,
    category: null,
    notes: null,
    ...overrides,
  };
}

/** Resolved record helper. */
function res(
  confidence: number,
  outcome: PredictionOutcome,
  extra: Partial<PredictionRecord> = {}
) {
  return rec({ confidence, outcome, resolvedAt: NOW, ...extra });
}

describe('normalizeConfidence / outcomeToNumber / binIndex', () => {
  it('clamps confidence into [0,1] and maps non-finite to 0', () => {
    expect(normalizeConfidence(-0.2)).toBe(0);
    expect(normalizeConfidence(1.5)).toBe(1);
    expect(normalizeConfidence(0.42)).toBe(0.42);
    expect(normalizeConfidence(NaN)).toBe(0);
  });

  it('maps outcomes to 1/0', () => {
    expect(outcomeToNumber('true')).toBe(1);
    expect(outcomeToNumber('false')).toBe(0);
  });

  it('assigns bins with 1.0 landing in bin 9, not an 11th bin', () => {
    expect(binIndex(0)).toBe(0);
    expect(binIndex(0.04)).toBe(0);
    expect(binIndex(0.1)).toBe(1);
    expect(binIndex(0.999)).toBe(9);
    expect(binIndex(1)).toBe(9);
  });
});

describe('brierScore', () => {
  it('is 0 for no resolved records', () => {
    expect(brierScore([])).toBe(0);
  });

  it('matches the hand-computed value', () => {
    // ((0.8-1)^2 + (0.3-0)^2)/2 = (0.04 + 0.09)/2 = 0.065
    const b = brierScore([res(0.8, 'true'), res(0.3, 'false')]);
    expect(b).toBeCloseTo(0.065, 12);
  });

  it('is 0 for perfect forecasts (p=1 true, p=0 false)', () => {
    expect(brierScore([res(1, 'true'), res(0, 'false')])).toBe(0);
  });

  it('is 1 for maximally-wrong forecasts (p=1 false, p=0 true)', () => {
    expect(brierScore([res(1, 'false'), res(0, 'true')])).toBe(1);
  });
});

describe('buildBins', () => {
  it('always returns 10 bins; empty bins carry zeros', () => {
    const bins = buildBins([res(0.95, 'true')]);
    expect(bins).toHaveLength(10);
    expect(bins[0].count).toBe(0);
    expect(bins[0].meanPredicted).toBe(0);
    expect(bins[0].hitRate).toBe(0);
    expect(bins[9].count).toBe(1);
    expect(bins[9].meanPredicted).toBeCloseTo(0.95, 12);
    expect(bins[9].hitRate).toBe(1);
  });

  it('computes per-bin meanPredicted and hitRate', () => {
    const bins = buildBins([res(0.9, 'true'), res(0.9, 'false')]);
    expect(bins[9].count).toBe(2);
    expect(bins[9].meanPredicted).toBeCloseTo(0.9, 12);
    expect(bins[9].hitRate).toBe(0.5);
  });
});

describe('baseRate / meanConfidence / overconfidence', () => {
  it('computes base rate and mean confidence', () => {
    const resolved = [res(0.8, 'true'), res(0.6, 'false'), res(0.4, 'false')];
    expect(baseRate(resolved)).toBeCloseTo(1 / 3, 12);
    expect(meanConfidence(resolved)).toBeCloseTo((0.8 + 0.6 + 0.4) / 3, 12);
  });

  it('reports positive overconfidence when predictions exceed reality', () => {
    const report = buildCalibrationReport([res(0.9, 'false'), res(0.9, 'false'), res(0.9, 'true')]);
    // mean confidence 0.9 vs base rate 1/3 -> strongly overconfident.
    expect(report.overconfidence).toBeGreaterThan(0);
  });

  it('reports negative overconfidence (underconfident) when reality exceeds predictions', () => {
    const report = buildCalibrationReport([res(0.2, 'true'), res(0.2, 'true'), res(0.2, 'false')]);
    expect(report.overconfidence).toBeLessThan(0);
  });
});

describe('murphyDecomposition identity', () => {
  // The identity brier == reliability - resolution + uncertainty holds exactly
  // when within-bin forecasts are equal (so reliability uses the bin mean).
  // These fixtures keep each occupied bin's confidences identical.
  const fixtures: PredictionRecord[][] = [
    [res(0.9, 'true'), res(0.9, 'true'), res(0.9, 'false'), res(0.2, 'false'), res(0.2, 'false')],
    [res(0.5, 'true'), res(0.5, 'false')],
    [res(0.7, 'true'), res(0.7, 'true'), res(0.7, 'true')], // all true (uncertainty 0)
    [res(0.1, 'false'), res(0.3, 'false'), res(0.6, 'true'), res(0.85, 'true')], // one per bin
  ];

  it('holds to 1e-9 across fixtures', () => {
    for (const f of fixtures) {
      const { reliability, resolution, uncertainty } = murphyDecomposition(f);
      const identity = reliability - resolution + uncertainty;
      expect(Math.abs(brierScore(f) - identity)).toBeLessThan(1e-9);
    }
  });

  it('is all-zero for no resolved records', () => {
    expect(murphyDecomposition([])).toEqual({ reliability: 0, resolution: 0, uncertainty: 0 });
  });

  it('has zero uncertainty when every outcome is identical', () => {
    expect(murphyDecomposition([res(0.7, 'true'), res(0.4, 'true')]).uncertainty).toBe(0);
  });
});

describe('buildTrend', () => {
  it('emits cumulative Brier ordered by resolvedAt then id', () => {
    const trend = buildTrend([
      res(0.8, 'true', { id: 'b', resolvedAt: NOW + 20 }),
      res(0.3, 'false', { id: 'a', resolvedAt: NOW + 10 }),
    ]);
    expect(trend.map(t => t.count)).toEqual([1, 2]);
    expect(trend[0].through).toBe(NOW + 10); // 'a' resolved first
    expect(trend[1].brierScore).toBeCloseTo(brierScore([res(0.3, 'false'), res(0.8, 'true')]), 12);
  });

  it('is empty when nothing is resolved', () => {
    expect(buildTrend([])).toEqual([]);
  });
});

describe('buildCalibrationReport', () => {
  it('returns an empty/zero report for no records', () => {
    const r = buildCalibrationReport([]);
    expect(r.resolvedCount).toBe(0);
    expect(r.openCount).toBe(0);
    expect(r.brierScore).toBe(0);
    expect(r.bins).toHaveLength(10);
    expect(r.trend).toEqual([]);
  });

  it('splits open vs resolved', () => {
    const r = buildCalibrationReport([res(0.8, 'true'), rec({ confidence: 0.5 })]);
    expect(r.resolvedCount).toBe(1);
    expect(r.openCount).toBe(1);
  });
});

describe('isOverdue', () => {
  it('flags open predictions past their resolveBy', () => {
    expect(isOverdue(rec({ resolveBy: NOW - 1 }), NOW)).toBe(true);
    expect(isOverdue(rec({ resolveBy: NOW + 1 }), NOW)).toBe(false);
    expect(isOverdue(rec({ resolveBy: null }), NOW)).toBe(false);
    // Resolved predictions are never overdue.
    expect(isOverdue(res(0.5, 'true', { resolveBy: NOW - 1 }), NOW)).toBe(false);
  });
});

describe('pure transforms', () => {
  it('addRecord appends without mutating', () => {
    const before: PredictionRecord[] = [];
    const after = addRecord(before, rec({ id: 'x' }));
    expect(before).toHaveLength(0);
    expect(after).toHaveLength(1);
  });

  it('resolveRecord sets outcome/resolvedAt and is a no-op on already-resolved', () => {
    const open = rec({ id: 'x' });
    const resolved = resolveRecord([open], 'x', 'true', NOW + 5);
    expect(resolved[0].outcome).toBe('true');
    expect(resolved[0].resolvedAt).toBe(NOW + 5);
    // Re-resolving does nothing.
    const again = resolveRecord(resolved, 'x', 'false', NOW + 9);
    expect(again[0].outcome).toBe('true');
    expect(again[0].resolvedAt).toBe(NOW + 5);
  });

  it('deleteRecord removes by id', () => {
    expect(deleteRecord([rec({ id: 'a' }), rec({ id: 'b' })], 'a').map(r => r.id)).toEqual(['b']);
  });
});

describe('parsePredictionRecords', () => {
  it('returns [] for non-arrays', () => {
    expect(parsePredictionRecords(null)).toEqual([]);
    expect(parsePredictionRecords({})).toEqual([]);
    expect(parsePredictionRecords('nope')).toEqual([]);
  });

  it('keeps valid records and drops malformed ones', () => {
    const parsed = parsePredictionRecords([
      {
        id: 'a',
        statement: 'x',
        confidence: 0.7,
        createdAt: NOW,
        outcome: 'true',
        resolvedAt: NOW,
      },
      { id: 'b', statement: 'y', confidence: 1.4, createdAt: NOW }, // confidence clamped
      { id: 'c', confidence: 0.5, createdAt: NOW }, // missing statement -> dropped
      { statement: 'no id', confidence: 0.5, createdAt: NOW }, // missing id -> dropped
      'garbage',
      null,
    ]);
    expect(parsed.map(p => p.id)).toEqual(['a', 'b']);
    expect(parsed[1].confidence).toBe(1); // clamped from 1.4
    expect(parsed[0].outcome).toBe('true');
  });

  it('coerces an invalid outcome to null (open)', () => {
    const parsed = parsePredictionRecords([
      { id: 'a', statement: 'x', confidence: 0.5, createdAt: NOW, outcome: 'maybe' },
    ]);
    expect(parsed[0].outcome).toBeNull();
  });
});
