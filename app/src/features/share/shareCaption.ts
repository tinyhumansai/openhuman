/**
 * LLM-backed headline drafting for the share cards feature (#5006).
 *
 * Reuses the existing one-shot completion RPC (`openhuman.inference_agent_chat_simple`)
 * to turn an agent's chat output into a short, punchy, first-person headline for
 * the card. Every failure mode (RPC error, empty/garbage response, no model
 * configured) degrades gracefully to the deterministic `buildFallbackHeadline`
 * so the Share flow never blocks on inference.
 *
 * PRIVACY: the prompt is built from the assistant message the user is already
 * looking at, and the response is scrubbed by `sanitizeHeadline` before use.
 */
import { callCoreRpc } from '../../services/coreRpcClient';
import { CORE_RPC_METHODS } from '../../services/rpcMethods';
import { buildFallbackHeadline, redactSensitive, sanitizeHeadline } from './shareContent';

const LOG_PREFIX = '[share-caption]';

/** How much of the agent output we feed the model. Keeps the prompt cheap. */
const MAX_CONTEXT_CHARS = 1200;

function buildPrompt(agentOutput: string): string {
  const context = redactSensitive(agentOutput).slice(0, MAX_CONTEXT_CHARS);
  return [
    'You write short, punchy social-media headlines.',
    'Read the AI assistant output below and write ONE first-person headline',
    '(max 90 characters) describing what the AI agent just accomplished for its user.',
    'No hashtags, no quotation marks, no emoji, no trailing period. Reply with only the headline.',
    '',
    'Assistant output:',
    context,
  ].join('\n');
}

/**
 * Extracts the response text from whatever envelope shape the core returns for
 * `inference_agent_chat_simple`. The op wraps its `String` result via
 * `RpcOutcome::single_log`, which serialises to `{ result, logs }`; older/other
 * transports may hand back a bare string or a `{ response }` object. We accept
 * all three defensively.
 */
export function extractResponseText(raw: unknown): string {
  if (typeof raw === 'string') return raw;
  if (raw && typeof raw === 'object') {
    const obj = raw as Record<string, unknown>;
    for (const key of ['result', 'response', 'data', 'text']) {
      if (typeof obj[key] === 'string') return obj[key] as string;
    }
  }
  return '';
}

/**
 * Drafts a card headline for `agentOutput`. Resolves to an LLM headline when
 * available, otherwise the deterministic fallback. Never rejects.
 *
 * @param agentOutput the assistant message text the user is sharing.
 * @param threadId    optional thread id, forwarded for inference-log grouping.
 */
export async function draftShareHeadline(agentOutput: string, threadId?: string): Promise<string> {
  const fallback = buildFallbackHeadline(agentOutput);
  const trimmed = agentOutput.trim();
  if (!trimmed) return fallback;

  try {
    console.debug(`${LOG_PREFIX} requesting draft len=${trimmed.length} thread=${threadId ?? '-'}`);
    const raw = await callCoreRpc<unknown>({
      method: CORE_RPC_METHODS.inferenceAgentChatSimple,
      params: {
        message: buildPrompt(trimmed),
        temperature: 0.6,
        ...(threadId ? { thread_id: threadId } : {}),
      },
    });
    const headline = sanitizeHeadline(extractResponseText(raw));
    if (headline) {
      console.debug(`${LOG_PREFIX} draft ok len=${headline.length}`);
      return headline;
    }
    console.debug(`${LOG_PREFIX} draft empty after sanitize; using fallback`);
    return fallback;
  } catch (err) {
    console.debug(
      `${LOG_PREFIX} draft failed; using fallback err_type=${
        err instanceof Error ? err.name : typeof err
      }`
    );
    return fallback;
  }
}
