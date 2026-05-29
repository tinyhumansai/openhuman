/**
 * Predicate Diversity — pure Shannon-entropy engine over the predicate vocabulary.
 *
 * The memory knowledge graph is a set of (subject)-[predicate]->(object)
 * triples. Where every other lens analyses the GRAPH (who points at whom), this
 * lens analyses the VOCABULARY (what relations the graph speaks in). It answers:
 *
 *   - How VARIED is the relation vocabulary the assistant has built — does it
 *     speak in dozens of distinct predicates or just two or three?
 *   - How EVEN is the use of that vocabulary — does one predicate dominate, or
 *     are the predicates used with roughly equal frequency?
 *
 * The classical information-theoretic answers are Shannon entropy
 *   H = -Σ p_i · log2(p_i)
 * over the distribution of predicates, and the normalised evenness H / log2(D)
 * (where D is the number of distinct predicates). A graph with one predicate
 * used 1000 times has the same NUMBER of relations as a graph with 10 predicates
 * used 100 times each, but only the latter carries 3.32 bits of vocabulary
 * surprise per relation; the former carries 0. No degree, PageRank, eccentricity
 * or coreness metric captures this.
 *
 * Everything here is PURE and DETERMINISTIC: no React, no RPC, no clock, no
 * randomness. To keep the entropy sum BYTE-identical across input permutations
 * (floating-point addition is not associative and the p·log2(p) summands are
 * not exactly representable), the running sum walks predicates in their
 * CANONICAL sorted order — never in input or Map-iteration order. The same
 * graph therefore always yields identical numbers, and every branch is
 * unit-testable.
 *
 * Load-bearing design choices (do not "fix" without reading the tests):
 *   - Self-loops, parallel edges, and multi-document evidence are NOT collapsed —
 *     this is a frequency analysis over RELATIONS, not over the undirected
 *     simple-graph structure used by the topology lenses.
 *   - A relation with a non-string predicate is dropped entirely (matches the
 *     malformed-row handling of the sibling engines).
 *   - With zero or one distinct predicates, maxEntropy is defined as 0 (the
 *     mathematically-undefined log2(1)/log2(0) is filled with the conventional
 *     boundary), and evenness collapses to 0.
 */
import type { GraphRelation } from '../../utils/tauriCommands/memory';

export interface PredicateStats {
  predicate: string;
  count: number; // number of relations with this predicate
  frequency: number; // count / totalRelations (in [0,1]); 0 when totalRelations === 0
}

export interface DiversityResult {
  predicates: PredicateStats[]; // sorted count DESC, then predicate ASC
  totalRelations: number; // total relations counted (non-string predicate dropped)
  distinctPredicates: number; // number of distinct predicates
  entropy: number; // Shannon entropy in bits, H = -Σ p·log2(p)
  maxEntropy: number; // log2(distinctPredicates), 0 when D <= 1
  evenness: number; // entropy / maxEntropy, in [0,1]; 0 when maxEntropy === 0
}

/** Compute predicate-distribution diversity statistics. PURE. */
export function computePredicateDiversity(relations: GraphRelation[]): DiversityResult {
  const counts = new Map<string, number>();
  let totalRelations = 0;
  for (const relation of relations) {
    if (typeof relation.predicate !== 'string') continue;
    counts.set(relation.predicate, (counts.get(relation.predicate) ?? 0) + 1);
    totalRelations += 1;
  }

  // Canonical predicate-ASC ordering so the entropy sum is byte-identical
  // regardless of input order (float addition is not associative).
  const sortedKeys = [...counts.keys()].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
  const distinctPredicates = sortedKeys.length;

  let entropy = 0;
  if (totalRelations > 0) {
    for (const key of sortedKeys) {
      const p = (counts.get(key) ?? 0) / totalRelations;
      if (p > 0) entropy -= p * Math.log2(p);
    }
  }

  const maxEntropy = distinctPredicates <= 1 ? 0 : Math.log2(distinctPredicates);
  const evenness = maxEntropy === 0 ? 0 : entropy / maxEntropy;

  const predicates: PredicateStats[] = sortedKeys
    .map(key => {
      const count = counts.get(key) ?? 0;
      return {
        predicate: key,
        count,
        frequency: totalRelations === 0 ? 0 : count / totalRelations,
      };
    })
    .sort((a, b) => {
      if (b.count !== a.count) return b.count - a.count;
      return a.predicate < b.predicate ? -1 : a.predicate > b.predicate ? 1 : 0;
    });

  return { predicates, totalRelations, distinctPredicates, entropy, maxEntropy, evenness };
}
