import {
  type ToolCallMessagePart,
  type ToolCallMessagePartComponent,
  useAui,
} from '@assistant-ui/react';
import { useCallback } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { useAuiThreadId } from '../../../providers/AssistantUiRuntimeProvider';
import { decideApproval } from '../../../services/api/approvalApi';
import {
  clearPendingApprovalForThread,
  type PendingApproval,
} from '../../../store/chatRuntimeSlice';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { ApprovalCardAdapter } from '../aui/ApprovalCardAdapter';
import { ElicitationAdapter } from '../aui/ElicitationAdapter';
import { PermissionGrantAdapter } from '../aui/PermissionGrantAdapter';
import { isApprovalPending, OpenHumanToolCall } from './AssistantUiToolCall';

/**
 * Resolve the store's parked request for this part, or `null` when the part is
 * not the one the gate is holding.
 *
 * The part carries only the request id and the decision options — everything a
 * human needs to *read* before deciding (`message`, the extracted `command`)
 * lives on `PendingApproval` in Redux, because assistant-ui's `approval` type
 * has nowhere to put a request summary: its `reason` field is the reason a
 * decision was given, not the reason one is being asked for.
 */
function useGatedApproval(
  approval: ToolCallMessagePart['approval']
): { threadId: string; request: PendingApproval } | null {
  const threadId = useAuiThreadId();
  const request = useAppSelector(state =>
    threadId ? (state.chatRuntime.pendingApprovalByThread?.[threadId] ?? null) : null
  );
  if (!threadId || !request) return null;
  if (request.requestId !== approval?.id) return null;
  return { threadId, request };
}

/** The tool that parks on the ApprovalGate but needs OAuth, not approve/deny. */
const COMPOSIO_CONNECT_TOOL = 'composio_connect';

/**
 * A parked `composio_connect` call.
 *
 * It arrives over the same `approval_request` path as every other gated tool,
 * but "Approve" is the wrong affordance: approving without connecting resumes
 * the agent against a toolkit that still has no credentials. The existing
 * connect card runs the OAuth handoff, polls until the toolkit is live, and
 * only then resolves the gate with `approve_once` (or `deny` on cancel/timeout)
 * — so it is reused verbatim rather than reimplemented against
 * `respondToApproval`.
 *
 * Falls through to the ordinary card once the approval is resolved, or when the
 * request is not the one the store holds: `PendingApproval.toolkit` names the
 * integration to connect and lives in Redux, not on the part.
 */
const ComposioConnectCall: ToolCallMessagePartComponent = props => {
  const gate = useGatedApproval(props.approval);
  if (!gate) return <OpenHumanToolCall {...props} />;
  return (
    // Keyed by request id so a second parked connect remounts the card with
    // fresh phase / field / poll state, matching the legacy placement.
    <PermissionGrantAdapter
      key={gate.request.requestId}
      threadId={gate.threadId}
      approval={gate.request}
    />
  );
};

/** Redacted args the gate extracted for display, as the approval card's command. */
function commandFromApproval(approval: PendingApproval): string {
  return approval.command ?? '';
}

/**
 * A parked tool call, with the decision attached to the call it gates.
 *
 * The controls are `ApprovalRequestCard` — the surface AGENTS.md designates for
 * the approval gate — rather than a bar of our own. That is the whole point: the
 * card renders the core's `Run <tool> — <summary>` explanation and the exact
 * command above its buttons, so the summary and the decision cannot come apart.
 * A bespoke bar had already drifted from the card once, showing "Always allow"
 * for a `shell` call whose command the user could not read — and
 * `approve_always_for_tool` writes the auto-approve allowlist, so that blind
 * decision would have been a durable one.
 *
 * The card resolves the gate itself, so the runtime's `respondToApproval` has
 * no caller here. `onRespondToToolApproval` on the external-store adapter is
 * still required and must not be deleted as dead: the part declares a pending
 * approval, so any assistant-ui renderer mounted on this runtime can answer it
 * — including the kit's own `ToolFallback`, which `thread.tsx` falls back to
 * whenever no override is supplied — and that call throws without it.
 */
const GatedToolCall: ToolCallMessagePartComponent = props => {
  const gate = useGatedApproval(props.approval);
  const { t } = useT();
  const dispatch = useAppDispatch();
  if (!gate) return <OpenHumanToolCall {...props} />;
  const { threadId, request } = gate;
  return (
    <OpenHumanToolCall
      {...props}
      approvalCard={
        <div className="px-3 pb-3">
          {/* Keyed by request id so a second parked request remounts the card
              with fresh decision/error state, matching the legacy placement. */}
          <ApprovalCardAdapter
            key={request.requestId}
            ariaLabel={t('chat.approval.title')}
            title={t('chat.approval.title')}
            subtitle={request.message || t('chat.approval.fallback')}
            command={commandFromApproval(request)}
            toolName={request.toolName}
            expiresAt={request.expiresAt}
            alwaysDecision="approve_always_for_tool"
            alwaysHint={t('chat.approval.alwaysAllowHint')}
            analyticsPrefix="chat-approval"
            onDecide={async decision => {
              await decideApproval(request.requestId, decision);
              dispatch(clearPendingApprovalForThread({ threadId }));
            }}
          />
        </div>
      }
    />
  );
};

/** The top-level clarification tool: never approval-gated, just waits on the user. */
const ASK_USER_CLARIFICATION_TOOL = 'ask_user_clarification';

/**
 * A top-level `ask_user_clarification` call — the agent itself (not a
 * delegated sub-agent, which `SubagentTaskCard` already renders its own
 * question UI for) needs a structured answer before the turn can continue.
 * Answered the same way a sub-agent's clarification is: append
 * an ordinary user turn through the runtime (see `ElicitationAdapter`'s doc
 * comment for why there is no separate RPC to call instead).
 */
const ElicitationCall: ToolCallMessagePartComponent = ({ args, result }) => {
  const aui = useAui();
  const { t } = useT();
  const question = (args as { question?: string } | undefined)?.question ?? '';
  const answer = useCallback(
    (text: string) => {
      void aui.thread.append({ role: 'user', content: [{ type: 'text', text }] });
    },
    [aui]
  );
  // Declining is an answer too: the run is parked until a user turn arrives,
  // so a Decline that sent nothing left the turn waiting forever. It sends a
  // plain "carry on without it" reply the orchestrator reads like any other.
  const decline = useCallback(() => {
    answer(t('chat.elicitation.declineReply'));
  }, [answer, t]);
  return (
    <ElicitationAdapter
      server="OpenHuman"
      message={question}
      pending={result === undefined}
      onAnswer={answer}
      onDecline={decline}
      testId="assistant-ui-elicitation"
    />
  );
};

/**
 * Route every call the toolkit does not own through an assistant-ui-native
 * rich renderer.
 *
 * `task` used to be special-cased here; it is now a `defineToolkit` entry
 * (`aui/toolkit.tsx`) registered on the runtime provider's `config`, so
 * assistant-ui resolves it before this fallback ever mounts. Every other tool
 * name — the vast majority, since most are dynamic (shell, file ops, MCP,
 * Composio, web search, ...) and cannot be enumerated in a static registry —
 * still comes through here, which is also where the approval gate,
 * `composio_connect` routing, and the top-level clarification question live:
 * all three are keyed on the part's own fields (`approval`, `toolName`), not
 * on a static registry entry, so no per-name registry entry could own them
 * without duplicating this same check in every entry.
 *
 * The gated branches are chosen on the part's own `approval` field, before any
 * component that reads Redux is mounted. An ordinary tool call therefore never
 * subscribes to the store — and, less obviously, still renders on a surface
 * that has no store at all, which is how most of the tool-card tests mount it.
 */
export const ChatToolFallback: ToolCallMessagePartComponent = props => {
  if (!isApprovalPending(props.approval)) {
    if (props.toolName === ASK_USER_CLARIFICATION_TOOL) return <ElicitationCall {...props} />;
    return <OpenHumanToolCall {...props} />;
  }
  if (props.toolName === COMPOSIO_CONNECT_TOOL) return <ComposioConnectCall {...props} />;
  return <GatedToolCall {...props} />;
};
