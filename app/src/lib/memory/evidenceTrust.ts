/**
 * Evidence-Weighted Trust Lens — pure trust-and-corroboration engine.
 *
 * The 20 shipped intelligence lenses all read the (subject, predicate, object)
 * triple structure or its document provenance. None of them makes
 * `evidenceCount` — the field that measures how many times an assertion has
 * been independently corroborated — the PRIMARY signal. This lens does.
 *
 * It surfaces three orthogonal-to-everything-shipped numbers:
 *
 *   - TRUST QUOTIENT per entity (TQ) — the mean evidence per relation an
 *     entity is involved in. A prolific entity with many single-witness
 *     facts has low TQ; a quiet entity whose few relations are heavily
 *     corroborated has high TQ. Degree alone (centrality, k-core) cannot
 *     express this distinction.
 *   - EVIDENCE GINI (G) — the inequality of evidence concentration across
 *     entities. 0 means every entity carries the same weight of evidence;
 *     near 1 means one entity dominates. Healthy memories sit in the
 *     middle; runaway concentration suggests a single source is being
 *     over-trusted.
 *   - UNDER-CORROBORATED WORKLIST — relations whose evidence falls below
 *     a graph-derived threshold (max(1, floor(median_positive_evidence / 4))),
 *     handed to the curator as a "review these next" queue.
 *
 * Two reliability ranks fall out as supporting outputs:
 *
 *   - Predicate Reliability Index (PRI) per predicate — mean evidence per
 *     relation of that predicate kind. Surfaces predicates that are
 *     systematically thin (a `birthdate` you read once vs a `knows` you
 *     re-heard a dozen times).
 *   - `degraded: true` when every relation has evidence === 1 (the field
 *     was never populated meaningfully), so the UI can show an empty-state
 *     instead of misleading the user with constant TQ everywhere.
 *
 * Everything here is PURE and DETERMINISTIC. The only float divisions are
 * single-op (TQ, PRI, Gini final ratio) on integer numerators, so there is
 * no float-add-associativity hazard. All Map iteration is replaced by
 * canonical sorted-key walks (entities by JSON.stringify([namespace, name])
 * ASC; predicates by predicate ASC; relations by canonical relKey ASC).
 *
 * Load-bearing design choices (do not "fix" without reading the tests):
 *   - evidenceCount is COERCED via `Number(...)` (handles string-encoded
 *     JSON numerics like "3" / "1e2") then SANITISED: any non-finite, NaN,
 *     or non-positive value collapses to 0. Sanitised values are floored
 *     to integers so all subsequent sums are exact integer arithmetic.
 *   - Relations with sanitised evidence 0 are KEPT (they still represent
 *     an assertion; we flag them in the under-corroborated bucket). They
 *     contribute 0 to W(x) and Σw.
 *   - Multigraph evidence policy: when the same (namespace, subject,
 *     predicate, object) tuple appears multiple times in the input
 *     (different documentIds, etc.), evidence SUMS across all instances.
 *     The unique tuple counts as ONE distinct relation for D(x), PRI's
 *     denominator, and the under-corroborated worklist.
 *   - Entity identity is (namespace, name) — a name appearing in two
 *     namespaces produces two distinct rows. We deliberately avoid the
 *     case-folding / trimming reserved for the Entity Duplicates lens.
 *   - Gini uses the closed-form `G = Σ_i (2i - n - 1) w_i / (n · Σw)` with
 *     1-indexed i over sorted-ascending weights — one integer-weighted sum
 *     and one division, so the result is order-immune.
 *   - Median of positive evidence picks the LOWER MIDDLE index for
 *     even-sized lists (NOT the arithmetic mean — that would introduce
 *     float drift). Ties on evidence are broken by relKey ASC so the
 *     median is byte-identical across input permutations.
 *   - Empty-graph and all-zero-evidence guards: globalGini = 0 when
 *     entityCount < 2 OR Σw === 0 (the OR is critical — `&&` would crash
 *     on an all-NaN graph).
 */
import type { GraphRelation } from '../../utils/tauriCommands/memory';

export interface EntityTrust {
  entity: string;
  namespace: string | null;
  weightOfEvidence: number; // W(x)
  degree: number; // D(x) — distinct relations touching x
  trustQuotient: number; // W(x) / D(x); 0 when D(x) === 0
}

export interface PredicateReliability {
  predicate: string;
  weightSum: number; // total sanitised evidence on relations of this predicate
  relationCount: number; // distinct relations of this predicate
  reliability: number; // weightSum / relationCount; 0 when relationCount === 0
}

export interface UnderCorroboratedRelation {
  namespace: string | null;
  subject: string;
  predicate: string;
  object: string;
  evidence: number; // sanitised value (Math.floor of positive finite number, else 0)
}

export interface EvidenceTrustResult {
  entities: EntityTrust[]; // sorted trustQuotient DESC, weightOfEvidence DESC, namespace ASC, entity ASC
  predicates: PredicateReliability[]; // sorted reliability DESC, predicate ASC
  underCorroborated: UnderCorroboratedRelation[]; // sorted evidence ASC, then relKey ASC
  totalRelations: number; // distinct (namespace, s, p, o) tuples after multigraph collapse
  totalEvidence: number; // Σ sanitised evidence across all distinct relations
  entityCount: number; // distinct (namespace, name) entities
  globalGini: number; // 0..1; 0 when entityCount < 2 OR totalEvidence === 0
  threshold: number; // max(1, floor(median_positive_evidence / 4)); 1 when no positives
  degraded: boolean; // true when every distinct relation has sanitised evidence === 1
}

function isRelation(relation: GraphRelation): boolean {
  return (
    typeof relation.subject === 'string' &&
    typeof relation.predicate === 'string' &&
    typeof relation.object === 'string'
  );
}

/** Coerce + sanitise an evidenceCount value. Anything non-finite, NaN, or
 * non-positive collapses to 0; otherwise floored to an integer. Handles the
 * common JSON quirks: string-encoded numerics, scientific notation, undefined. */
function sanitiseEvidence(raw: unknown): number {
  const numeric = typeof raw === 'number' ? raw : Number(raw);
  if (!Number.isFinite(numeric) || numeric <= 0) return 0;
  return Math.floor(numeric);
}

function entityKey(namespace: string | null, name: string): string {
  return JSON.stringify([namespace, name]);
}

function relKey(
  namespace: string | null,
  subject: string,
  predicate: string,
  object: string
): string {
  return JSON.stringify([namespace, subject, predicate, object]);
}

function compareStrings(a: string, b: string): number {
  if (a === b) return 0;
  return a < b ? -1 : 1;
}

interface DistinctRelation {
  namespace: string | null;
  subject: string;
  predicate: string;
  object: string;
  evidence: number;
  relKey: string;
}

/** Compute the Evidence-Weighted Trust Lens result. PURE. */
export function computeEvidenceTrust(relations: GraphRelation[]): EvidenceTrustResult {
  // Pass 1: multigraph collapse. Same (namespace, s, p, o) tuple appearing
  // multiple times SUMS evidence; it counts as ONE distinct relation.
  const distinctRels = new Map<string, DistinctRelation>();
  for (const relation of relations) {
    if (!isRelation(relation)) continue;
    const namespace = typeof relation.namespace === 'string' ? relation.namespace : null;
    const { subject, predicate, object } = relation;
    const key = relKey(namespace, subject, predicate, object);
    const evidence = sanitiseEvidence(relation.evidenceCount);
    const existing = distinctRels.get(key);
    if (existing === undefined) {
      distinctRels.set(key, { namespace, subject, predicate, object, evidence, relKey: key });
    } else {
      existing.evidence += evidence;
    }
  }

  // Canonical iteration order over distinct relations.
  const sortedRels = [...distinctRels.values()].sort((a, b) => compareStrings(a.relKey, b.relKey));

  // Pass 2: aggregate per-entity (W, D) and per-predicate (weightSum, count).
  const weightByEntity = new Map<string, number>();
  const degreeByEntity = new Map<string, number>();
  const weightByPredicate = new Map<string, number>();
  const countByPredicate = new Map<string, number>();
  let totalEvidence = 0;

  for (const rel of sortedRels) {
    // Every relation credits BOTH endpoints with the relation's evidence and
    // one degree count (undirected-degree trust-attribution convention — both
    // sides of a corroborated fact get the same trust credit). A self-loop
    // (subject === object) collapses both updates onto the same Map entry,
    // naturally doubling the touch — exactly the right behaviour, no special
    // case needed.
    const subjectKey = entityKey(rel.namespace, rel.subject);
    const objectKey = entityKey(rel.namespace, rel.object);

    weightByEntity.set(subjectKey, (weightByEntity.get(subjectKey) ?? 0) + rel.evidence);
    degreeByEntity.set(subjectKey, (degreeByEntity.get(subjectKey) ?? 0) + 1);
    weightByEntity.set(objectKey, (weightByEntity.get(objectKey) ?? 0) + rel.evidence);
    degreeByEntity.set(objectKey, (degreeByEntity.get(objectKey) ?? 0) + 1);

    weightByPredicate.set(
      rel.predicate,
      (weightByPredicate.get(rel.predicate) ?? 0) + rel.evidence
    );
    countByPredicate.set(rel.predicate, (countByPredicate.get(rel.predicate) ?? 0) + 1);

    totalEvidence += rel.evidence;
  }

  // Build per-entity rows in canonical entity-key sorted order.
  const sortedEntityKeys = [...weightByEntity.keys()].sort(compareStrings);
  const entities: EntityTrust[] = sortedEntityKeys.map(key => {
    const [namespace, entity] = JSON.parse(key) as [string | null, string];
    const w = weightByEntity.get(key) ?? 0;
    const d = degreeByEntity.get(key) ?? 0;
    return {
      entity,
      namespace,
      weightOfEvidence: w,
      degree: d,
      trustQuotient: d === 0 ? 0 : w / d,
    };
  });
  entities.sort((a, b) => {
    if (b.trustQuotient !== a.trustQuotient) return b.trustQuotient - a.trustQuotient;
    if (b.weightOfEvidence !== a.weightOfEvidence) return b.weightOfEvidence - a.weightOfEvidence;
    const ns = compareStrings(a.namespace ?? '', b.namespace ?? '');
    if (ns !== 0) return ns;
    return compareStrings(a.entity, b.entity);
  });

  // Per-predicate reliability rows in canonical predicate-ASC order.
  const sortedPredicates = [...weightByPredicate.keys()].sort(compareStrings);
  const predicates: PredicateReliability[] = sortedPredicates.map(predicate => {
    const weightSum = weightByPredicate.get(predicate) ?? 0;
    const relationCount = countByPredicate.get(predicate) ?? 0;
    return {
      predicate,
      weightSum,
      relationCount,
      reliability: relationCount === 0 ? 0 : weightSum / relationCount,
    };
  });
  predicates.sort((a, b) => {
    if (b.reliability !== a.reliability) return b.reliability - a.reliability;
    return compareStrings(a.predicate, b.predicate);
  });

  // Global Gini coefficient via the closed form on sorted entity weights:
  //   G = Σ_i (2i - n - 1) · w_i  /  (n · Σw)        (1-indexed i over w_i ASC)
  // Σw here is the sum of ENTITY weights (each relation's evidence is credited
  // to both subject and object, so Σw = 2·totalEvidence in the simple-graph
  // case but the entity-weight sum is what the Gini formula actually needs).
  const weightsAsc = entities.map(e => e.weightOfEvidence).sort((a, b) => a - b);
  let weightsSum = 0;
  for (const w of weightsAsc) weightsSum += w;
  let globalGini = 0;
  if (weightsAsc.length >= 2 && weightsSum > 0) {
    let weighted = 0;
    for (let i = 0; i < weightsAsc.length; i += 1) {
      weighted += (2 * (i + 1) - weightsAsc.length - 1) * weightsAsc[i];
    }
    globalGini = weighted / (weightsAsc.length * weightsSum);
  }

  // Median-positive-evidence threshold for the under-corroborated worklist.
  // Sort by (evidence ASC, relKey ASC) and pick the LOWER MIDDLE index for
  // even-sized lists — index `(n - 1) >> 1` — so the median is byte-identical
  // across input permutations and never averages two values (which would
  // introduce float drift).
  const positives = sortedRels
    .filter(r => r.evidence > 0)
    .sort((a, b) => {
      if (a.evidence !== b.evidence) return a.evidence - b.evidence;
      return compareStrings(a.relKey, b.relKey);
    });
  const threshold =
    positives.length === 0
      ? 1
      : Math.max(1, Math.floor(positives[(positives.length - 1) >> 1].evidence / 4));

  // Under-corroborated relations: sanitised evidence < threshold.
  const underCorroborated: UnderCorroboratedRelation[] = sortedRels
    .filter(r => r.evidence < threshold)
    .map(r => ({
      namespace: r.namespace,
      subject: r.subject,
      predicate: r.predicate,
      object: r.object,
      evidence: r.evidence,
    }))
    .sort((a, b) => {
      if (a.evidence !== b.evidence) return a.evidence - b.evidence;
      return compareStrings(
        relKey(a.namespace, a.subject, a.predicate, a.object),
        relKey(b.namespace, b.subject, b.predicate, b.object)
      );
    });

  // Degraded mode: every distinct relation has sanitised evidence === 1.
  const degraded = sortedRels.length > 0 && sortedRels.every(r => r.evidence === 1);

  return {
    entities,
    predicates,
    underCorroborated,
    totalRelations: sortedRels.length,
    totalEvidence,
    entityCount: entities.length,
    globalGini,
    threshold,
    degraded,
  };
}
