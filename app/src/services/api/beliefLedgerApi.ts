/**
 * Thin RPC facade for the Belief Ledger.
 *
 * Adds ZERO new core surface — it composes three already-shipped JSON-RPC
 * wrappers:
 *   - memoryGraphQuery   (openhuman.memory_graph_query)  — the data source
 *   - openhumanAgentChat (openhuman.agent_chat)          — optional NL explanation
 *   - memoryDocIngest    (openhuman.memory_doc_ingest)   — persist a user correction
 */
import debug from 'debug';

import {
  type BeliefHistory,
  normalizeText,
  type SaveCorrectionParams,
} from '../../lib/memory/beliefLedger';
import { openhumanAgentChat } from '../../utils/tauriCommands/localAi';
import {
  type GraphRelation,
  memoryDocIngest,
  memoryGraphQuery,
} from '../../utils/tauriCommands/memory';

const log = debug('belief-ledger:api');

/** Load the knowledge-graph relations that feed the ledger. */
export async function loadRelations(namespace?: string): Promise<GraphRelation[]> {
  log('loadRelations namespace=%s', namespace ?? '(all)');
  const relations = await memoryGraphQuery(namespace);
  log('loadRelations -> %d relations', relations.length);
  return relations;
}

/**
 * English language names for the supported UI locales, used to add a
 * "respond in X" directive so the explanation comes back in the user's
 * language. An explicit map (rather than Intl.DisplayNames) keeps the prompt
 * deterministic across Node/ICU versions, so it can be asserted exactly.
 */
const LANGUAGE_NAMES: Record<string, string> = {
  ko: 'Korean',
  'zh-CN': 'Chinese',
  hi: 'Hindi',
  es: 'Spanish',
  ar: 'Arabic',
  fr: 'French',
  bn: 'Bengali',
  pt: 'Portuguese',
  de: 'German',
  ru: 'Russian',
  id: 'Indonesian',
  it: 'Italian',
  pl: 'Polish',
};

/**
 * Build the deterministic prompt the chair model answers when the user asks
 * to explain a contradiction. Pure (no I/O, no clock) so it can be asserted
 * exactly in tests. When `locale` names a non-English UI language, a directive
 * asks the model to reply in that language (the English default adds nothing).
 */
export function buildExplanationPrompt(history: BeliefHistory, locale?: string): string {
  const lines: string[] = [];
  lines.push(
    `The knowledge graph recorded conflicting values over time for the claim "${history.subject} ${history.predicate} …".`
  );
  lines.push('Observed values, newest first:');
  history.revisions.forEach(rev => {
    const when = new Date(rev.version.updatedAt * 1000).toISOString().slice(0, 10);
    const tag = rev.isCurrent ? ' [current best belief]' : '';
    lines.push(
      `- "${rev.version.object}" (${rev.kind}, seen ${when}, evidence x${rev.version.evidenceCount})${tag}`
    );
  });
  lines.push(
    'In 2-3 sentences, explain to the user why these differ over time and which value is most likely true now. Do not invent facts beyond what is listed.'
  );
  const language = locale ? LANGUAGE_NAMES[locale] : undefined;
  if (language) lines.push(`Respond in ${language}.`);
  return lines.join('\n');
}

/** Ask the agent for a natural-language explanation of a contested claim. */
export async function explainConflict(history: BeliefHistory, locale?: string): Promise<string> {
  const prompt = buildExplanationPrompt(history, locale);
  log('explainConflict claim=%s locale=%s', history.claimKey, locale ?? 'en');
  const response = await openhumanAgentChat(prompt);
  return response.result;
}

/**
 * Stable, content-derived ingest key so re-correcting the same claim overwrites
 * rather than piling up. Uses `encodeURIComponent(normalizeText(...))` for each
 * part: this keeps the key charset-safe WITHOUT a lossy ASCII slug that would
 * collapse distinct non-Latin or punctuation-only claims to the same empty
 * string and silently overwrite an unrelated correction.
 */
export function correctionKey(params: SaveCorrectionParams): string {
  const part = (s: string) => encodeURIComponent(normalizeText(s));
  return `belief-correction:${part(params.namespace)}:${part(params.subject)}:${part(params.predicate)}`;
}

/**
 * Persist a user's "this is the truth" correction as an llm_thought/correction
 * memory note, so the next `memoryGraphQuery` reflects the authoritative value.
 */
export async function saveCorrection(params: SaveCorrectionParams): Promise<void> {
  const key = correctionKey(params);
  // Log only the key — never the corrected value, which is exactly the kind of
  // sensitive user fact this feature handles (no PII in local/dev logs).
  log('saveCorrection key=%s', key);
  await memoryDocIngest({
    namespace: params.namespace,
    key,
    title: `Belief correction: ${params.subject} ${params.predicate}`,
    content: `The user confirmed the correct value: ${params.subject} ${params.predicate} ${params.correctObject}.`,
    source_type: 'llm_thought',
    category: 'correction',
    tags: ['belief-correction'],
    metadata: {
      subject: params.subject,
      predicate: params.predicate,
      object: params.correctObject,
    },
  });
}

export const beliefLedgerApi = {
  loadRelations,
  explainConflict,
  saveCorrection,
  buildExplanationPrompt,
  correctionKey,
};
