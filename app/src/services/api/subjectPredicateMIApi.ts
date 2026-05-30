/**
 * RPC facade for Subject-Predicate Mutual Information.
 *
 * Adds ZERO new core surface. It composes two already-shipped JSON-RPC wrappers:
 *   - memoryGraphQuery     (openhuman.memory_graph_query)     — the triples
 *   - memoryListNamespaces (openhuman.memory_list_namespaces) — the selector
 * and delegates all math to the pure, deterministic engine. Read-only: there is
 * no persistence — the result is always reproducible from the current graph.
 */
import debug from 'debug';

import {
  computeSubjectPredicateMI,
  type SubjectPredicateMIResult,
} from '../../lib/memory/subjectPredicateMI';
import { memoryGraphQuery, memoryListNamespaces } from '../../utils/tauriCommands/memory';

const log = debug('subject-predicate-mi:api');

/** Fetch graph relations for a namespace (or all) and compute MI + specialisation. */
export async function loadSubjectPredicateMI(
  namespace?: string
): Promise<SubjectPredicateMIResult> {
  const relations = await memoryGraphQuery(namespace);
  log('loadSubjectPredicateMI namespace=%s relations=%d', namespace ?? '(all)', relations.length);
  return computeSubjectPredicateMI(relations);
}

/** List the namespaces available for the namespace selector. */
export async function loadNamespaces(): Promise<string[]> {
  return memoryListNamespaces();
}

export const subjectPredicateMIApi = { loadSubjectPredicateMI, loadNamespaces };
