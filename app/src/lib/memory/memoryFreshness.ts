/**
 * Knowledge Freshness — pure decay-scoring engine.
 *
 * Every fact the assistant remembers is a (subject)-[predicate]->(object) triple
 * that was last reinforced at `updatedAt`. Human memory of an un-rehearsed fact
 * decays along a forgetting curve; this engine applies the same idea to the
 * assistant's stored facts so the UI can surface what is going STALE and should
 * be re-confirmed, rather than treating every stored fact as equally certain.
 *
 * recall(t) = 2 ^ (-ageDays / halfLifeDays)
 *   - ageDays      = days since the fact was last reinforced (updatedAt)
 *   - halfLifeDays = DEFAULT_HALF_LIFE_DAYS * (1 + log2(max(1, evidenceCount)))
 *
 * A fact corroborated by more evidence decays more slowly (a longer half-life),
 * with diminishing returns (log2). recall is 1.0 the moment a fact is recorded
 * and approaches 0 as it ages without reinforcement.
 *
 * Everything here is PURE and DETERMINISTIC. The engine never reads the clock:
 * the reference time `nowSeconds` is injected by the caller, so the same inputs
 * always yield the same report and every branch is unit-testable.
 */
import type { GraphRelation } from '../../utils/tauriCommands/memory';

export type FreshnessStatus = 'fresh' | 'fading' | 'stale';

export interface FactFreshness {
  id: string; // stable composite key (subject/predicate/object), JSON-encoded
  subject: string;
  predicate: string;
  object: string;
  evidenceCount: number;
  updatedAt: number; // epoch seconds the fact was last reinforced
  ageDays: number; // days since updatedAt (>= 0)
  halfLifeDays: number; // evidence-scaled half-life
  recall: number; // 0..1 recall probability now
  status: FreshnessStatus;
}

export interface FreshnessReport {
  facts: FactFreshness[]; // all facts, most stale first (recall asc, id asc)
  staleQueue: FactFreshness[]; // non-fresh facts only, same order (re-confirm queue)
  freshCount: number;
  fadingCount: number;
  staleCount: number;
  total: number;
  averageRecall: number; // mean recall across all facts (0 when none)
}

export interface FreshnessOptions {
  halfLifeDays?: number; // base half-life for an evidenceCount of 1
  freshThreshold?: number; // recall >= this => 'fresh'
  fadingThreshold?: number; // recall >= this (and < fresh) => 'fading', else 'stale'
}

export const DEFAULT_HALF_LIFE_DAYS = 30;
export const FRESH_THRESHOLD = 0.7;
export const FADING_THRESHOLD = 0.3;
const SECONDS_PER_DAY = 86400;

/** Evidence multiplier on the half-life: more corroboration decays slower. */
export function strengthFactor(evidenceCount: number): number {
  const ec = Number.isFinite(evidenceCount) && evidenceCount > 1 ? evidenceCount : 1;
  return 1 + Math.log2(ec);
}

/** Recall probability for a given age and half-life; clamped to [0, 1]. */
export function recallProbability(ageDays: number, halfLifeDays: number): number {
  if (!(halfLifeDays > 0)) return ageDays <= 0 ? 1 : 0;
  const age = ageDays > 0 ? ageDays : 0;
  const recall = 2 ** (-age / halfLifeDays);
  if (recall > 1) return 1;
  if (recall < 0) return 0;
  return recall;
}

/** Classify a recall probability into a freshness band. */
export function classify(
  recall: number,
  freshThreshold = FRESH_THRESHOLD,
  fadingThreshold = FADING_THRESHOLD
): FreshnessStatus {
  if (recall >= freshThreshold) return 'fresh';
  if (recall >= fadingThreshold) return 'fading';
  return 'stale';
}

/** Stable, collision-free key for a triple (no raw separators). */
function factKey(subject: string, predicate: string, object: string): string {
  return JSON.stringify([subject, predicate, object]);
}

/**
 * Compute the freshness report. Pure function of (relations, nowSeconds).
 * Duplicate triples are collapsed to the freshest occurrence (max updatedAt,
 * then max evidenceCount) so a fact is scored once at its strongest signal.
 */
export function computeFreshness(
  relations: GraphRelation[],
  nowSeconds: number,
  options: FreshnessOptions = {}
): FreshnessReport {
  const baseHalfLife = options.halfLifeDays ?? DEFAULT_HALF_LIFE_DAYS;
  const freshThreshold = options.freshThreshold ?? FRESH_THRESHOLD;
  const fadingThreshold = options.fadingThreshold ?? FADING_THRESHOLD;

  // 1. Collapse duplicate triples to their freshest, strongest occurrence.
  const bestByKey = new Map<string, GraphRelation>();
  for (const relation of relations) {
    const { subject, predicate, object } = relation;
    if (
      typeof subject !== 'string' ||
      typeof predicate !== 'string' ||
      typeof object !== 'string'
    ) {
      continue;
    }
    const key = factKey(subject, predicate, object);
    const existing = bestByKey.get(key);
    if (
      !existing ||
      relation.updatedAt > existing.updatedAt ||
      (relation.updatedAt === existing.updatedAt && relation.evidenceCount > existing.evidenceCount)
    ) {
      bestByKey.set(key, relation);
    }
  }

  // 2. Score each fact.
  const facts: FactFreshness[] = [];
  let recallSum = 0;
  let freshCount = 0;
  let fadingCount = 0;
  let staleCount = 0;
  for (const [key, relation] of bestByKey) {
    const evidenceCount =
      Number.isFinite(relation.evidenceCount) && relation.evidenceCount > 0
        ? relation.evidenceCount
        : 1;
    const halfLifeDays = baseHalfLife * strengthFactor(evidenceCount);
    const ageDays = Math.max(0, (nowSeconds - relation.updatedAt) / SECONDS_PER_DAY);
    const recall = recallProbability(ageDays, halfLifeDays);
    const status = classify(recall, freshThreshold, fadingThreshold);
    recallSum += recall;
    if (status === 'fresh') freshCount += 1;
    else if (status === 'fading') fadingCount += 1;
    else staleCount += 1;
    facts.push({
      id: key,
      subject: relation.subject,
      predicate: relation.predicate,
      object: relation.object,
      evidenceCount,
      updatedAt: relation.updatedAt,
      ageDays,
      halfLifeDays,
      recall,
      status,
    });
  }

  // 3. Sort most-stale-first (recall asc), stable id tie-break.
  facts.sort((a, b) => a.recall - b.recall || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));

  const total = facts.length;
  return {
    facts,
    staleQueue: facts.filter(f => f.status !== 'fresh'),
    freshCount,
    fadingCount,
    staleCount,
    total,
    averageRecall: total === 0 ? 0 : recallSum / total,
  };
}
