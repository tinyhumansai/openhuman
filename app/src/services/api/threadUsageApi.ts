import { callCoreRpc } from '../coreRpcClient';

/** One sub-agent archetype's contribution within a thread. */
export interface ThreadSubagentUsage {
  agentId: string;
  inputTokens: number;
  outputTokens: number;
  costUsd: number;
  runs: number;
}

/** Camel-cased per-thread usage totals consumed by the composer footer. */
export interface ThreadTokenUsage {
  threadId: string;
  inputTokens: number;
  outputTokens: number;
  cachedInputTokens: number;
  costUsd: number;
  turnCount: number;
  lastTurnInputTokens: number;
  lastTurnOutputTokens: number;
  contextWindow: number;
  model: string | null;
  updated: string | null;
  hasUsage: boolean;
  /** Per-archetype sub-agent breakdown (already included in the totals above). */
  subagents: ThreadSubagentUsage[];
}

/** Wire shape returned by `openhuman.threads_token_usage` (snake_case). */
interface ThreadSubagentUsageWire {
  agent_id: string;
  input_tokens: number;
  output_tokens: number;
  cost_usd: number;
  runs: number;
}

interface ThreadTokenUsageWire {
  thread_id: string;
  input_tokens: number;
  output_tokens: number;
  cached_input_tokens: number;
  cost_usd: number;
  turn_count: number;
  last_turn_input_tokens: number;
  last_turn_output_tokens: number;
  context_window: number;
  model: string | null;
  updated: string | null;
  has_usage: boolean;
  subagents?: ThreadSubagentUsageWire[];
}

interface Envelope<T> {
  data?: T;
}

/**
 * Fetch a thread's persisted token/cost totals from the core (read back from
 * its session transcripts). Returns zeros with `hasUsage: false` for a thread
 * that has no completed turns yet.
 */
export async function fetchThreadTokenUsage(threadId: string): Promise<ThreadTokenUsage> {
  const response = await callCoreRpc<Envelope<ThreadTokenUsageWire>>({
    method: 'openhuman.threads_token_usage',
    params: { thread_id: threadId },
  });
  const d = response?.data;
  if (!d) throw new Error('threads_token_usage returned an empty envelope');
  return {
    threadId: d.thread_id,
    inputTokens: d.input_tokens,
    outputTokens: d.output_tokens,
    cachedInputTokens: d.cached_input_tokens,
    costUsd: d.cost_usd,
    turnCount: d.turn_count,
    lastTurnInputTokens: d.last_turn_input_tokens,
    lastTurnOutputTokens: d.last_turn_output_tokens,
    contextWindow: d.context_window,
    model: d.model,
    updated: d.updated,
    hasUsage: d.has_usage,
    subagents: (d.subagents ?? []).map(s => ({
      agentId: s.agent_id,
      inputTokens: s.input_tokens,
      outputTokens: s.output_tokens,
      costUsd: s.cost_usd,
      runs: s.runs,
    })),
  };
}
