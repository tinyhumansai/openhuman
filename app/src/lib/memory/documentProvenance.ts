/**
 * Document Provenance Footprint — pure document-side bipartite engine.
 *
 * Every shipped lens to date analyses the (subject, predicate, object) TRIPLE
 * side of the graph. This lens pivots on the OTHER side: each relation also
 * carries the `documentIds` it was sourced from, projecting the corpus as a
 * DOCUMENT — ENTITY bipartite graph. It answers questions no triple-side lens
 * can:
 *
 *   - Which source documents are LOAD-BEARING (contribute entities or
 *     relations no other document also attests)?
 *   - Which documents most CORROBORATE each other (overlap their entity sets)?
 *   - What fraction of the user's memory is single-sourced — and therefore
 *     fragile if that source is removed or rejected?
 *   - How much of the graph has NO provenance at all?
 *
 * Everything here is PURE and DETERMINISTIC: no React, no RPC, no clock, no
 * randomness. All counts are integer; Jaccard ratios are computed once per
 * pair; output is sorted in a total order, so the same input always yields
 * byte-identical output regardless of relation insertion order or the order of
 * ids inside a single relation's `documentIds`.
 *
 * Load-bearing design choices (do not "fix" without reading the tests):
 *   - documentIds is NORMALISED on intake: non-array / non-string / empty-string
 *     entries are dropped, duplicates within a single relation are deduped (so
 *     documentIds: ['d1','d1'] is the same as ['d1']). That dedup is what makes
 *     `exclusiveRelations(d)` actually mean "every source agrees this is d's
 *     alone" instead of being fooled by a triple-attested d-only triple.
 *   - A relation whose normalised set is EMPTY is bucketed into a synthetic
 *     UNSOURCED cohort, counted as unsourcedRelationCount, and never appears
 *     as a "document" — so neither documents[] nor topOverlapPairs can be
 *     polluted by a sentinel id. The exposed counts let the UI surface
 *     "{N} relations have no provenance" without ever needing a stand-in.
 *   - Pair keys for the overlap matrix are computed with the canonical
 *     `a < b` ordering enforced AT INSERTION (not just at output sort), so
 *     the order of documentIds inside a relation cannot influence which
 *     pair is incremented.
 *   - Pairwise intersection is built from an INVERTED entity → docs index,
 *     not a quadratic D×D loop — so the per-pair cost is bounded by the
 *     number of entities the two documents actually share, not by the total
 *     document count.
 *   - relationKey is `JSON.stringify([subject, predicate, object])` so two
 *     triples whose strings together happen to look like a different triple
 *     cannot collide.
 *   - Entity / predicate / document identity is the raw string AS-IS: NO
 *     trimming, NO case-folding — matching the sibling lenses.
 *   - A relation with a non-string subject, predicate, or object is dropped
 *     entirely (matches the malformed-row handling of the sibling engines).
 */
import type { GraphRelation } from '../../utils/tauriCommands/memory';

export interface DocumentFootprint {
  documentId: string;
  entityCount: number; // distinct entities this document mentions
  relationCount: number; // distinct (s,p,o) triples this document attests
  exclusiveEntityCount: number; // entities reachable ONLY via this document
  exclusiveRelationCount: number; // relations whose every (deduped) documentId === this
  predicateCount: number; // distinct predicates this document attests
}

export interface OverlapPair {
  a: string; // lexicographically smaller document id
  b: string; // lexicographically larger document id
  jaccard: number; // intersection / union of entity sets, in [0, 1]
  intersectionSize: number; // |E_a ∩ E_b|
  unionSize: number; // |E_a ∪ E_b| (always > 0 in emitted pairs)
}

export interface ProvenanceResult {
  documents: DocumentFootprint[]; // sorted relationCount DESC, then documentId ASC
  topOverlapPairs: OverlapPair[]; // sorted jaccard DESC, intersectionSize DESC, a ASC, b ASC
  documentCount: number; // distinct sourced documents (excludes the unsourced bucket)
  totalRelations: number; // every relation surveyed (including unsourced)
  unsourcedRelationCount: number; // relations with empty/missing/non-array documentIds
  provenanceCoverage: number; // relationsWithDoc / totalRelations, in [0,1]; 0 when totalRelations === 0
  singleSourceRelationShare: number; // Σ exclusiveRelations(d) / relationsWithDoc; 0 when relationsWithDoc === 0
}

const MAX_OVERLAP_PAIRS = 50;

function isRelation(relation: GraphRelation): boolean {
  return (
    typeof relation.subject === 'string' &&
    typeof relation.predicate === 'string' &&
    typeof relation.object === 'string'
  );
}

/** Normalise `documentIds` into a deduped Set of non-empty string ids. */
function normaliseDocIds(documentIds: unknown): Set<string> {
  if (!Array.isArray(documentIds)) return new Set();
  const out = new Set<string>();
  for (const id of documentIds) {
    if (typeof id === 'string' && id.length > 0) out.add(id);
  }
  return out;
}

/** Compute the document-side provenance footprint of the memory graph. PURE. */
export function computeDocumentProvenance(relations: GraphRelation[]): ProvenanceResult {
  // Per-document accumulators.
  interface DocAccum {
    entities: Set<string>;
    relations: Set<string>;
    predicates: Set<string>;
    exclusiveRelationCount: number;
  }
  const accums = new Map<string, DocAccum>();
  // Inverted index entity -> set of docs that mention it, so per-pair
  // intersection can be tallied without ever materialising a D×D matrix.
  const entityToDocs = new Map<string, Set<string>>();

  let totalRelations = 0;
  let unsourcedRelationCount = 0;
  let relationsWithDoc = 0;

  for (const relation of relations) {
    if (!isRelation(relation)) continue;
    totalRelations += 1;

    const docs = normaliseDocIds(relation.documentIds);
    if (docs.size === 0) {
      unsourcedRelationCount += 1;
      continue;
    }
    relationsWithDoc += 1;

    const { subject, predicate, object } = relation;
    const relationKey = JSON.stringify([subject, predicate, object]);

    for (const doc of docs) {
      let accum = accums.get(doc);
      if (accum === undefined) {
        accum = {
          entities: new Set<string>(),
          relations: new Set<string>(),
          predicates: new Set<string>(),
          exclusiveRelationCount: 0,
        };
        accums.set(doc, accum);
      }
      accum.entities.add(subject);
      accum.entities.add(object);
      accum.relations.add(relationKey);
      accum.predicates.add(predicate);

      // Inverted index — subject and object both indexed under this doc.
      let subjectDocs = entityToDocs.get(subject);
      if (subjectDocs === undefined) {
        subjectDocs = new Set<string>();
        entityToDocs.set(subject, subjectDocs);
      }
      subjectDocs.add(doc);
      let objectDocs = entityToDocs.get(object);
      if (objectDocs === undefined) {
        objectDocs = new Set<string>();
        entityToDocs.set(object, objectDocs);
      }
      objectDocs.add(doc);
    }

    // Exclusive-relation credit: only when the relation's full deduped doc set
    // is a single doc. (A triple sourced by [d1, d1] dedupes to {d1} and IS
    // exclusive; a triple sourced by [d1, d2] is not.)
    if (docs.size === 1) {
      const onlyDoc = docs.values().next().value as string;
      const accum = accums.get(onlyDoc);
      if (accum !== undefined) accum.exclusiveRelationCount += 1;
    }
  }

  // Exclusive entities: entities whose entry in the inverted index points at
  // exactly one document.
  const exclusiveEntityCount = new Map<string, number>();
  for (const [, docs] of entityToDocs) {
    if (docs.size === 1) {
      const onlyDoc = docs.values().next().value as string;
      exclusiveEntityCount.set(onlyDoc, (exclusiveEntityCount.get(onlyDoc) ?? 0) + 1);
    }
  }

  // Pairwise intersection via the inverted index. For each entity, every
  // unordered pair of docs that share it bumps that pair's intersection. The
  // pair key is canonical (smaller id first) so insertion is order-immune.
  const pairIntersection = new Map<string, number>();
  for (const [, docs] of entityToDocs) {
    if (docs.size < 2) continue;
    const sortedDocs = [...docs].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
    for (let i = 0; i < sortedDocs.length; i += 1) {
      for (let j = i + 1; j < sortedDocs.length; j += 1) {
        const key = JSON.stringify([sortedDocs[i], sortedDocs[j]]);
        pairIntersection.set(key, (pairIntersection.get(key) ?? 0) + 1);
      }
    }
  }

  // Materialise per-document footprints.
  const documents: DocumentFootprint[] = [];
  let totalExclusiveRelations = 0;
  for (const [documentId, accum] of accums) {
    documents.push({
      documentId,
      entityCount: accum.entities.size,
      relationCount: accum.relations.size,
      exclusiveEntityCount: exclusiveEntityCount.get(documentId) ?? 0,
      exclusiveRelationCount: accum.exclusiveRelationCount,
      predicateCount: accum.predicates.size,
    });
    totalExclusiveRelations += accum.exclusiveRelationCount;
  }
  documents.sort((a, b) => {
    if (b.relationCount !== a.relationCount) return b.relationCount - a.relationCount;
    return a.documentId < b.documentId ? -1 : a.documentId > b.documentId ? 1 : 0;
  });

  // Materialise overlap pairs.
  const sizeByDoc = new Map<string, number>();
  for (const doc of documents) sizeByDoc.set(doc.documentId, doc.entityCount);

  const pairs: OverlapPair[] = [];
  for (const [key, intersectionSize] of pairIntersection) {
    if (intersectionSize === 0) continue;
    const [a, b] = JSON.parse(key) as [string, string];
    const sizeA = sizeByDoc.get(a) ?? 0;
    const sizeB = sizeByDoc.get(b) ?? 0;
    const unionSize = sizeA + sizeB - intersectionSize;
    if (unionSize === 0) continue;
    pairs.push({ a, b, jaccard: intersectionSize / unionSize, intersectionSize, unionSize });
  }
  pairs.sort((p, q) => {
    if (q.jaccard !== p.jaccard) return q.jaccard - p.jaccard;
    if (q.intersectionSize !== p.intersectionSize) return q.intersectionSize - p.intersectionSize;
    if (p.a !== q.a) return p.a < q.a ? -1 : 1;
    return p.b < q.b ? -1 : p.b > q.b ? 1 : 0;
  });
  const topOverlapPairs = pairs.slice(0, MAX_OVERLAP_PAIRS);

  return {
    documents,
    topOverlapPairs,
    documentCount: documents.length,
    totalRelations,
    unsourcedRelationCount,
    provenanceCoverage: totalRelations === 0 ? 0 : relationsWithDoc / totalRelations,
    singleSourceRelationShare:
      relationsWithDoc === 0 ? 0 : totalExclusiveRelations / relationsWithDoc,
  };
}
