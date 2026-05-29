/**
 * Belief Ledger — temporal contradiction & belief-revision engine.
 *
 * Reconstructs, per (subject, predicate) claim, how what the agent believes
 * about the user changed over time. Generic memory products treat memory as
 * append-only or last-write-wins; this engine instead recognizes when newer
 * observations *contradict* or *supersede* older ones about the same claim,
 * picks a current best belief, and exposes the full revision history so the
 * user can audit and correct it.
 *
 * Everything here is a pure, deterministic transform over `GraphRelation[]`:
 * no React, no RPC, no `Date.now()` / `Math.random()`. The reference time is
 * always injected (`nowSeconds`), so the same input always yields the same
 * output and every branch is unit-testable without a clock, network, or DB.
 */
import type { GraphRelation } from '../../utils/tauriCommands';

export type RevisionKind = 'reinforced' | 'refined' | 'superseded' | 'contradicted';

/** One observed object-value for a claim, carried from a source GraphRelation. */
export interface RelationVersion {
  /** Raw object value, as stored. */
  object: string;
  /** Normalized object value used for equality / conflict detection. */
  normalizedObject: string;
  /** Epoch seconds the relation was last updated. */
  updatedAt: number;
  /** Evidence count, clamped to >= 0 (NaN treated as 0). */
  evidenceCount: number;
  namespace: string | null;
  documentIds: string[];
  chunkIds: string[];
  /** From `attrs.entity_types.subject` (mirrors MemoryInsights), else null. */
  subjectType: string | null;
  /** From `attrs.entity_types.object`, else null. */
  objectType: string | null;
}

export interface BeliefRevision {
  version: RelationVersion;
  kind: RevisionKind;
  /** True when this version is the resolved current belief. */
  isCurrent: boolean;
}

export interface BeliefHistory {
  /** `${normSubject}␟${normPredicate}` — stable, collision-free key. */
  claimKey: string;
  /** Display subject (first-seen raw subject for this claim). */
  subject: string;
  /** Display predicate (first-seen raw predicate for this claim). */
  predicate: string;
  /** Predicate is in the mutually-exclusive set (a value change is a contradiction). */
  mutuallyExclusive: boolean;
  /** >= 2 distinct normalized object values were observed. */
  hasConflict: boolean;
  /** The resolved current best belief. */
  current: RelationVersion;
  /** Best version among the *other* distinct values, or null when only one value. */
  runnerUp: RelationVersion | null;
  /** Score-margin between winner and runner-up, in [0, 1]. 1 when unrivalled. */
  confidence: number;
  /** Revisions, newest -> oldest. */
  revisions: BeliefRevision[];
  /** Number of distinct normalized object values. */
  distinctValueCount: number;
  /** Sum of evidence across all versions (used for stable output ordering). */
  totalEvidence: number;
}

export interface SaveCorrectionParams {
  namespace: string;
  subject: string;
  predicate: string;
  correctObject: string;
}

/**
 * Predicates whose object is single-valued by nature: a new value doesn't
 * *add* to the old one, it *replaces* it — so a change is a genuine
 * contradiction over time rather than an additional fact. Stored normalized.
 */
export const MUTUALLY_EXCLUSIVE_PREDICATES: ReadonlySet<string> = new Set([
  'lives in',
  'works at',
  'works for',
  'located in',
  'reports to',
  'prefers',
  'uses',
  'married to',
  'born in',
  'member of',
  'manages',
  'assigned to',
]);

const SEP = '␟'; // U+241F SYMBOL FOR UNIT SEPARATOR — cannot appear in content.
const DEFAULT_HALF_LIFE_DAYS = 180;
const SECONDS_PER_DAY = 86_400;

/** Lowercase, dashes/underscores -> spaces, collapse whitespace, trim. */
export function normalizeText(s: string): string {
  return s.toLowerCase().replace(/[_-]/g, ' ').replace(/\s+/g, ' ').trim();
}

/** `normalizeText` plus stripping trailing sentence punctuation. */
export function normalizeObject(s: string): string {
  return normalizeText(s)
    .replace(/[.!?,;:]+$/, '')
    .trim();
}

/** Stable per-claim key from the (subject, predicate) pair. */
export function normalizeClaimKey(subject: string, predicate: string): string {
  return `${normalizeText(subject)}${SEP}${normalizeText(predicate)}`;
}

export function isMutuallyExclusive(predicate: string): boolean {
  return MUTUALLY_EXCLUSIVE_PREDICATES.has(normalizeText(predicate));
}

function clampEvidence(value: number): number {
  return Number.isFinite(value) && value > 0 ? value : 0;
}

function readEntityType(rel: GraphRelation, side: 'subject' | 'object'): string | null {
  const entityTypes = (rel.attrs?.entity_types ?? {}) as Record<string, unknown>;
  const value = entityTypes[side];
  return typeof value === 'string' && value.length > 0 ? value : null;
}

/**
 * Bucket relations by normalized (subject, predicate). Relations with an
 * empty subject / predicate / object (after trim) are skipped — they carry
 * no claim. Evidence is clamped to >= 0.
 */
export function groupRelationsByClaim(relations: GraphRelation[]): Map<string, RelationVersion[]> {
  const groups = new Map<string, RelationVersion[]>();
  for (const rel of relations) {
    if (!rel.subject?.trim() || !rel.predicate?.trim() || !rel.object?.trim()) {
      continue;
    }
    const normalizedObject = normalizeObject(rel.object);
    // A value that is empty AFTER normalization (e.g. punctuation-only like
    // "..." or "?!") carries no belief — skip it so it can't become a phantom
    // value or a spurious "conflict" against a real one.
    if (!normalizedObject) {
      continue;
    }
    const key = normalizeClaimKey(rel.subject, rel.predicate);
    const version: RelationVersion = {
      object: rel.object,
      normalizedObject,
      updatedAt: Number.isFinite(rel.updatedAt) ? rel.updatedAt : 0,
      evidenceCount: clampEvidence(rel.evidenceCount),
      namespace: rel.namespace ?? null,
      documentIds: Array.isArray(rel.documentIds) ? rel.documentIds : [],
      chunkIds: Array.isArray(rel.chunkIds) ? rel.chunkIds : [],
      subjectType: readEntityType(rel, 'subject'),
      objectType: readEntityType(rel, 'object'),
    };
    const bucket = groups.get(key);
    if (bucket) bucket.push(version);
    else groups.set(key, [version]);
  }
  return groups;
}

export function detectConflicts(versions: RelationVersion[]): {
  hasConflict: boolean;
  distinctValueCount: number;
} {
  const distinct = new Set(versions.map(v => v.normalizedObject));
  return { hasConflict: distinct.size >= 2, distinctValueCount: distinct.size };
}

/**
 * Recency x evidence score. Recency decays with a 180-day half-life so a
 * fresh observation outweighs a stale one; evidence adds a sub-linear
 * (logarithmic) boost so repeated corroboration matters without letting an
 * old, heavily-cited value win forever. `nowSeconds` is injected.
 */
export function scoreVersion(
  v: RelationVersion,
  nowSeconds: number,
  halfLifeDays: number = DEFAULT_HALF_LIFE_DAYS
): number {
  const ageSeconds = Math.max(0, nowSeconds - v.updatedAt);
  const recency = Math.pow(0.5, ageSeconds / (halfLifeDays * SECONDS_PER_DAY));
  const evidence = Math.log(v.evidenceCount + 1);
  return recency * (1 + evidence);
}

/**
 * Pick the current best belief and the strongest contender from the *other*
 * distinct values. Deterministic tie-breaks: score desc -> updatedAt desc ->
 * evidence desc -> normalizedObject ascending.
 */
export function resolveCurrentBelief(
  versions: RelationVersion[],
  nowSeconds: number
): { current: RelationVersion; runnerUp: RelationVersion | null } {
  if (versions.length === 0) {
    throw new Error('resolveCurrentBelief: versions must be non-empty');
  }
  const ranked = [...versions].sort((a, b) => compareForResolution(a, b, nowSeconds));
  const current = ranked[0];
  const runnerUp = ranked.find(v => v.normalizedObject !== current.normalizedObject) ?? null;
  return { current, runnerUp };
}

function compareForResolution(a: RelationVersion, b: RelationVersion, nowSeconds: number): number {
  const sa = scoreVersion(a, nowSeconds);
  const sb = scoreVersion(b, nowSeconds);
  if (sb !== sa) return sb - sa;
  if (b.updatedAt !== a.updatedAt) return b.updatedAt - a.updatedAt;
  if (b.evidenceCount !== a.evidenceCount) return b.evidenceCount - a.evidenceCount;
  return a.normalizedObject < b.normalizedObject
    ? -1
    : a.normalizedObject > b.normalizedObject
      ? 1
      : 0;
}

/**
 * Confidence = normalized score margin between winner and runner-up, in
 * [0, 1]. 1 when there is no rival value; ~0 for a near-tie.
 */
export function computeConfidence(
  current: RelationVersion,
  runnerUp: RelationVersion | null,
  nowSeconds: number
): number {
  if (!runnerUp) return 1;
  const sc = scoreVersion(current, nowSeconds);
  const sr = scoreVersion(runnerUp, nowSeconds);
  const denom = sc + sr;
  // Both scores can underflow to 0 for extremely old timestamps (recency
  // decays toward 0); `!(denom > 0)` also rejects NaN so we never divide by 0.
  if (!(denom > 0)) return 0;
  const margin = Math.min(1, Math.max(0, (sc - sr) / denom));
  return Number(margin.toFixed(4));
}

/**
 * True when one value extends/narrows the other: every word of the shorter
 * value also appears in the longer value (token-subset). This is a far better
 * proxy for "refinement" than raw substring containment, which produces false
 * positives on incidental overlap (e.g. "go" inside "mango").
 */
function isTokenRefinement(a: string, b: string): boolean {
  const ta = a.split(' ').filter(Boolean);
  const tb = b.split(' ').filter(Boolean);
  if (ta.length === 0 || tb.length === 0) return false;
  const [shorter, longer] = ta.length <= tb.length ? [ta, tb] : [tb, ta];
  const longerSet = new Set(longer);
  return shorter.every(token => longerSet.has(token));
}

/**
 * Classify how a single version relates to the belief timeline.
 * - `reinforced`: same value as the current best belief (or as its immediate predecessor).
 * - `refined`: the first observation, or a value that extends/narrows its
 *   predecessor (token-subset — see {@link isTokenRefinement}).
 * - `contradicted`: a value change on a mutually-exclusive predicate.
 * - `superseded`: any other value change.
 */
export function classifyRevision(
  version: RelationVersion,
  current: RelationVersion,
  prevByTime: RelationVersion | null,
  mutuallyExclusive: boolean
): RevisionKind {
  if (version.normalizedObject === current.normalizedObject) return 'reinforced';
  if (!prevByTime) return 'refined';
  if (prevByTime.normalizedObject === version.normalizedObject) return 'reinforced';
  if (mutuallyExclusive) return 'contradicted';
  if (isTokenRefinement(version.normalizedObject, prevByTime.normalizedObject)) return 'refined';
  return 'superseded';
}

/** Chronological ascending: updatedAt asc -> evidence asc -> normalizedObject asc. */
function compareChronoAsc(a: RelationVersion, b: RelationVersion): number {
  if (a.updatedAt !== b.updatedAt) return a.updatedAt - b.updatedAt;
  if (a.evidenceCount !== b.evidenceCount) return a.evidenceCount - b.evidenceCount;
  return a.normalizedObject < b.normalizedObject
    ? -1
    : a.normalizedObject > b.normalizedObject
      ? 1
      : 0;
}

/**
 * Build the full set of belief histories from raw relations. Output is
 * ordered most-contested first: hasConflict desc, then confidence ascending
 * (lowest-confidence / most-contested surfaced first), then total evidence
 * desc. `nowSeconds` is injected for deterministic scoring.
 */
export function buildBeliefHistories(
  relations: GraphRelation[],
  nowSeconds: number
): BeliefHistory[] {
  const groups = groupRelationsByClaim(relations);
  const histories: BeliefHistory[] = [];

  // First-seen raw (subject, predicate) per claim, captured in a single pass
  // (mirrors groupRelationsByClaim's skip rules) so display lookup is O(1) per
  // claim instead of re-scanning all relations for each one.
  const displayByKey = new Map<string, { subject: string; predicate: string }>();
  for (const rel of relations) {
    if (!rel.subject?.trim() || !rel.predicate?.trim() || !rel.object?.trim()) continue;
    if (!normalizeObject(rel.object)) continue;
    const key = normalizeClaimKey(rel.subject, rel.predicate);
    if (!displayByKey.has(key)) {
      displayByKey.set(key, { subject: rel.subject, predicate: rel.predicate });
    }
  }

  for (const [claimKey, versions] of groups) {
    const { hasConflict, distinctValueCount } = detectConflicts(versions);
    const { current, runnerUp } = resolveCurrentBelief(versions, nowSeconds);
    const confidence = computeConfidence(current, runnerUp, nowSeconds);
    const mutuallyExclusive = isMutuallyExclusive(claimKeyPredicate(claimKey));

    // Classify each version against its chronological predecessor.
    const chronoAsc = [...versions].sort(compareChronoAsc);
    const kindByVersion = new Map<RelationVersion, RevisionKind>();
    chronoAsc.forEach((version, i) => {
      const prev = i > 0 ? chronoAsc[i - 1] : null;
      kindByVersion.set(version, classifyRevision(version, current, prev, mutuallyExclusive));
    });

    // Display newest -> oldest.
    const revisions: BeliefRevision[] = [...versions]
      .sort((a, b) => -compareChronoAsc(a, b))
      .map(version => ({
        version,
        kind: kindByVersion.get(version) ?? 'refined',
        isCurrent: version === current,
      }));

    const totalEvidence = versions.reduce((sum, v) => sum + v.evidenceCount, 0);
    const display = displayByKey.get(claimKey) ?? splitClaimKey(claimKey);

    histories.push({
      claimKey,
      subject: display.subject,
      predicate: display.predicate,
      mutuallyExclusive,
      hasConflict,
      current,
      runnerUp,
      confidence,
      revisions,
      distinctValueCount,
      totalEvidence,
    });
  }

  histories.sort((a, b) => {
    if (a.hasConflict !== b.hasConflict) return a.hasConflict ? -1 : 1;
    if (a.confidence !== b.confidence) return a.confidence - b.confidence;
    return b.totalEvidence - a.totalEvidence;
  });
  return histories;
}

/** Recover the normalized predicate half of a claim key. */
function claimKeyPredicate(claimKey: string): string {
  const idx = claimKey.indexOf(SEP);
  return idx >= 0 ? claimKey.slice(idx + 1) : claimKey;
}

/** Degraded display fallback: split a claim key back into its two halves. */
function splitClaimKey(claimKey: string): { subject: string; predicate: string } {
  const idx = claimKey.indexOf(SEP);
  return idx >= 0
    ? { subject: claimKey.slice(0, idx), predicate: claimKey.slice(idx + 1) }
    : { subject: claimKey, predicate: '' };
}
