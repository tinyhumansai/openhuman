import type { WorkflowProposal } from '../../store/chatRuntimeSlice';
import type { ThreadMessage } from '../../types/thread';

/**
 * Shared parsing for `workflow_proposal` payloads produced by the Rust
 * `propose_workflow` / `revise_workflow` / `edit_workflow` tools.
 *
 * Four delivery channels funnel through here so they can never drift:
 *  1. live `tool_result` socket events (main-agent tool call),
 *  2. live `subagent_tool_result` socket events (`build_workflow` delegate),
 *  3. persisted thread messages whose `extraMetadata.scope` is
 *     `workflow_proposal` — the durable backstop written by the Rust core
 *     when an async `workflow_builder` run completes, which lets the card
 *     rehydrate after a reload or a dropped socket event.
 *  4. direct `openhuman.flows_build` API responses used by the workflow
 *     builder UI.
 *
 * IMPORTANT: match on the payload's `type` field, NOT on tool names. This
 * mirrors the Rust `flows_build` path's `extract_workflow_proposal`, which
 * also scans tool results by payload `type` — a name allowlist here can
 * silently drop proposals from newly added tools (as happened when
 * `edit_workflow` was added without updating an earlier list).
 */
export function coerceWorkflowProposal(parsed: unknown): WorkflowProposal | null {
  if (!parsed || typeof parsed !== 'object') return null;
  const obj = parsed as Record<string, unknown>;
  if (obj.type !== 'workflow_proposal') return null;
  if (typeof obj.name !== 'string' || obj.graph == null) return null;

  const summary = (obj.summary ?? {}) as Record<string, unknown>;
  const rawSteps = Array.isArray(summary.steps) ? summary.steps : [];
  const steps = rawSteps
    .filter((s): s is Record<string, unknown> => !!s && typeof s === 'object')
    .map(s => ({
      kind: typeof s.kind === 'string' ? s.kind : 'unknown',
      name: typeof s.name === 'string' ? s.name : '',
      config_hint: typeof s.config_hint === 'string' ? s.config_hint : undefined,
    }));

  return {
    name: obj.name,
    graph: obj.graph,
    // The Rust tool defaults `require_approval` to `true` when the caller
    // omits it, so treat anything other than an explicit `false` as `true`
    // here too — keeps the client's fallback in lockstep with the server's.
    requireApproval: obj.require_approval !== false,
    summary: { trigger: typeof summary.trigger === 'string' ? summary.trigger : '', steps },
  };
}

export function parseWorkflowProposal(output: string): WorkflowProposal | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(output);
  } catch {
    return null;
  }
  return coerceWorkflowProposal(parsed);
}

export function maybeParseWorkflowProposalTool(
  _toolName: string,
  success: boolean,
  output: string | undefined
): WorkflowProposal | null {
  if (!success || !output) return null;
  return parseWorkflowProposal(output);
}

/**
 * Rehydrate the newest unconsumed workflow proposal from a thread's persisted
 * messages. The Rust core appends a message with
 * `extraMetadata: { scope: 'workflow_proposal', proposal, task_id, ... }`
 * when an async `workflow_builder` run finishes; once the user saves or
 * dismisses the card the message is marked `consumed: true` so it does not
 * resurrect on the next load.
 */
export function extractWorkflowProposalFromMessages(
  messages: ThreadMessage[]
): WorkflowProposal | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    const meta = messages[i].extraMetadata;
    if (!meta || meta.scope !== 'workflow_proposal' || meta.consumed === true) continue;
    const proposal = coerceWorkflowProposal(meta.proposal);
    if (proposal) {
      return { ...proposal, sourceMessageId: messages[i].id };
    }
  }
  return null;
}
