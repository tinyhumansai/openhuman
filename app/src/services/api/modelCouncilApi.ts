/**
 * Typed RPC wrapper for the Model Council domain.
 *
 * Calls `openhuman.model_council_run` — runs a question through several
 * "member" models in parallel, then a "chair" model synthesizes their
 * answers (surfacing agreement / disagreement / unique insight).
 *
 * The core returns the result inside a `{ result, logs }` CLI envelope
 * (the handler attaches a completion log), so `runCouncil` unwraps it
 * defensively and returns the bare `ModelCouncilResult`.
 */
import debug from 'debug';

import { callCoreRpc } from '../coreRpcClient';

const log = debug('model-council:api');

/** One member model's contribution. `response` and `error` are mutually exclusive. */
export interface CouncilMemberResult {
  /** The model id this seat ran. */
  model: string;
  /** The model's answer, or `null` if the call failed. */
  response: string | null;
  /** The failure message, or `null` on success. */
  error: string | null;
}

/** Full council result: every member's answer plus the chair synthesis. */
export interface ModelCouncilResult {
  question: string;
  members: CouncilMemberResult[];
  chair_model: string;
  synthesis: string;
}

export interface RunCouncilParams {
  question: string;
  /** Member model ids or "default" seats to consult (repeated seats preserved; capped server-side). */
  member_models: string[];
  /** Model id that synthesizes the member answers. */
  chair_model: string;
  /** Optional sampling temperature applied to every call. */
  temperature?: number;
}

export interface RunCouncilMemberParams {
  question: string;
  model: string;
  temperature?: number;
}

export interface SynthesizeCouncilParams {
  question: string;
  members: CouncilMemberResult[];
  chair_model: string;
  temperature?: number;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return null;
  }
  return value as Record<string, unknown>;
}

/**
 * Unwrap the core's `{ result, logs }` CLI envelope. Returns the inner
 * `result` when the envelope shape is present, otherwise passes the payload
 * through unchanged — so the caller is correct whether or not the handler
 * attached logs.
 */
export function unwrapCouncilEnvelope(payload: unknown): ModelCouncilResult {
  const record = asRecord(payload);
  if (
    record &&
    'result' in record &&
    record.result != null &&
    'logs' in record &&
    Array.isArray(record.logs)
  ) {
    return record.result as ModelCouncilResult;
  }
  return payload as ModelCouncilResult;
}

export const modelCouncilApi = {
  answerMember: async (params: RunCouncilMemberParams): Promise<CouncilMemberResult> => {
    log('answer member question=%s model=%s', params.question.slice(0, 40), params.model);
    const payload = await callCoreRpc<unknown>({
      method: 'openhuman.model_council_answer_member',
      params,
      timeoutMs: 180_000,
    });
    return unwrapCouncilEnvelope(payload) as unknown as CouncilMemberResult;
  },

  synthesizeCouncil: async (params: SynthesizeCouncilParams): Promise<ModelCouncilResult> => {
    log(
      'synthesize question=%s members=%d chair=%s',
      params.question.slice(0, 40),
      params.members.length,
      params.chair_model
    );
    const payload = await callCoreRpc<unknown>({
      method: 'openhuman.model_council_synthesize',
      params,
      timeoutMs: 180_000,
    });
    return unwrapCouncilEnvelope(payload);
  },

  runCouncil: async (params: RunCouncilParams): Promise<ModelCouncilResult> => {
    log(
      'run question=%s members=%o chair=%s',
      params.question.slice(0, 40),
      params.member_models,
      params.chair_model
    );
    const payload = await callCoreRpc<unknown>({
      method: 'openhuman.model_council_run',
      params,
      // Member calls run in parallel but each is a full model round-trip plus
      // a synthesis pass — give it well beyond the default 30s before timing out.
      timeoutMs: 180_000,
    });
    const result = unwrapCouncilEnvelope(payload);
    log(
      'run done: %d members, synthesis %d chars',
      result.members?.length ?? 0,
      result.synthesis?.length ?? 0
    );
    return result;
  },
};
