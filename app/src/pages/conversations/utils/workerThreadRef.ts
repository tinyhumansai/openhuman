/**
 * Parses the `[worker_thread_ref]…[/worker_thread_ref]` envelope the
 * Rust core's `spawn_subagent` tool emits when it spawns a sub-agent
 * with `dedicated_thread: true`. The envelope is appended to the parent
 * thread's tool_result text so the UI can render a clickable card
 * linking to the new worker thread instead of dumping the sub-agent's
 * full transcript inline.
 */

export interface WorkerThreadRef {
  threadId: string;
  label: string;
  agentId?: string;
  taskId?: string;
  elapsedMs?: number;
  iterations?: number;
}

export interface ParsedWorkerThreadRef {
  /** The text that appeared before the envelope (model-readable summary). */
  before: string;
  /** The decoded reference, if the envelope parsed cleanly. */
  ref: WorkerThreadRef;
  /** The text that appeared after the envelope (rare but supported). */
  after: string;
}

const ENVELOPE_RE = /\[worker_thread_ref\]\s*\n?([\s\S]*?)\n?\s*\[\/worker_thread_ref\]/;

export function parseWorkerThreadRef(
  input: string | undefined | null
): ParsedWorkerThreadRef | null {
  if (!input) return null;
  const match = ENVELOPE_RE.exec(input);
  if (!match) return null;

  let payload: unknown;
  try {
    payload = JSON.parse(match[1].trim());
  } catch {
    return null;
  }
  if (!payload || typeof payload !== 'object') return null;
  const obj = payload as Record<string, unknown>;

  const threadId = typeof obj.thread_id === 'string' ? obj.thread_id.trim() : '';
  if (!threadId) return null;

  const label = typeof obj.label === 'string' && obj.label.trim().length > 0 ? obj.label : 'worker';

  return {
    before: input.slice(0, match.index).trim(),
    after: input.slice(match.index + match[0].length).trim(),
    ref: {
      threadId,
      label,
      agentId: typeof obj.agent_id === 'string' ? obj.agent_id : undefined,
      taskId: typeof obj.task_id === 'string' ? obj.task_id : undefined,
      elapsedMs: typeof obj.elapsed_ms === 'number' ? obj.elapsed_ms : undefined,
      iterations: typeof obj.iterations === 'number' ? obj.iterations : undefined,
    },
  };
}
