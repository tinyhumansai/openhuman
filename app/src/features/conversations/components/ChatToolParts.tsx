import {
  type ToolCallMessagePart,
  type ToolCallMessagePartComponent,
  useAui,
} from '@assistant-ui/react';
import { type FC, type PropsWithChildren, useCallback } from 'react';

import type { ThreadGroupPart } from '../../../components/assistant-ui/thread';
import {
  ToolGroupContent,
  ToolGroupRoot,
  ToolGroupTrigger,
} from '../../../components/assistant-ui/tool-group';
import ApprovalRequestCard from '../../../components/chat/ApprovalRequestCard';
import IntegrationConnectCard from '../../../components/chat/IntegrationConnectCard';
import { useAuiThreadId } from '../../../providers/AssistantUiRuntimeProvider';
import type { PendingApproval, SubagentActivity } from '../../../store/chatRuntimeSlice';
import { useAppSelector } from '../../../store/hooks';
import { AssistantUiSubagentCall, isActiveSubagentStatus } from './AssistantUiSubagentCall';
import { isApprovalPending, OpenHumanToolCall } from './AssistantUiToolCall';
import { useSubagentDrawerHost } from './aui/subagentDrawerHost';

function asSubagentActivity(value: unknown): SubagentActivity | undefined {
  if (!value || typeof value !== 'object') return undefined;
  const candidate = value as Partial<SubagentActivity>;
  if (
    typeof candidate.taskId !== 'string' ||
    typeof candidate.agentId !== 'string' ||
    !Array.isArray(candidate.toolCalls)
  ) {
    return undefined;
  }
  return candidate as SubagentActivity;
}

function readSubagentState(
  args: unknown,
  result: unknown
): { activity: SubagentActivity | undefined; running: boolean } {
  const completed = asSubagentActivity(result);
  // A settled part carries the activity, but "settled" is not "succeeded":
  // ask the activity's own status so a `failed` delegation is not rendered as
  // a completed one.
  if (completed) return { activity: completed, running: isActiveSubagentStatus(completed.status) };
  const progress =
    args && typeof args === 'object'
      ? asSubagentActivity((args as { progress?: unknown }).progress)
      : undefined;
  return { activity: progress, running: result === undefined };
}

/** Adapt an assistant-ui `task` part onto the shared delegation card. */
export const SubagentCall: ToolCallMessagePartComponent = ({ args, result }) => {
  const aui = useAui();
  const { activity, running } = readSubagentState(args, result);
  const description = (args as { description?: string } | undefined)?.description;
  const fallbackAgent = (args as { subagent_type?: string } | undefined)?.subagent_type;
  const resolved = activity ?? {
    taskId: 'pending-subagent',
    agentId: fallbackAgent ?? 'subagent',
    toolCalls: [],
  };
  // A delegation parked on `ask_user_clarification` is unblocked by an ordinary
  // user turn: the orchestrator is holding the `[SUBAGENT_AWAITING_USER]`
  // envelope and resumes the child with `continue_subagent` once the user
  // answers. Appending through the runtime routes to the external store's
  // `onNew` and out to the registered chat surface, i.e. the same entry point
  // as the composer's Send, so queueing behind an in-flight turn is decided in
  // one place rather than duplicated here.
  const answer = useCallback(
    (text: string) => {
      void aui.thread.append({ role: 'user', content: [{ type: 'text', text }] });
    },
    [aui]
  );
  // "View full processing" opens the host's `SubagentDrawer`. This is the only
  // renderer for a delegation on the assistant-ui surface, and it was the only
  // one that offered no way in: the legacy `ToolTimelineBlock` passes `onView`
  // per row, and the sole remaining launcher -- `BackgroundProcessesPanel` --
  // lists async/typed spawns only, so every other delegation's persisted worker
  // conversation was unreachable.
  //
  // Offered only when the host says the drawer can resolve the row -- it looks
  // the delegation up by `taskId` in the thread's live timeline
  // (`TranscriptOverlays`) and renders nothing for a `taskId` that is not
  // there, so a part replayed from the settled core transcript would otherwise
  // get a button that opens an empty sheet. Asked of the host rather than of
  // Redux directly: this component renders on surfaces that have no store.
  const drawerHost = useSubagentDrawerHost();
  const taskId = resolved.taskId;
  const view = useCallback(() => drawerHost?.open(taskId), [drawerHost, taskId]);
  return (
    <AssistantUiSubagentCall
      activity={resolved}
      running={running}
      description={description}
      onAnswer={answer}
      onView={drawerHost?.canOpen(taskId) ? view : undefined}
    />
  );
};

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
    <div data-testid="assistant-ui-integration-connect">
      {/* Keyed by request id so a second parked connect remounts the card with
          fresh phase / field / poll state, matching the legacy placement. */}
      <IntegrationConnectCard
        key={gate.request.requestId}
        threadId={gate.threadId}
        approval={gate.request}
      />
    </div>
  );
};

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
  if (!gate) return <OpenHumanToolCall {...props} />;
  return (
    <OpenHumanToolCall
      {...props}
      approvalCard={
        <div className="px-3 pb-3">
          {/* Keyed by request id so a second parked request remounts the card
              with fresh decision/error state, matching the legacy placement. */}
          <ApprovalRequestCard
            key={gate.request.requestId}
            threadId={gate.threadId}
            approval={gate.request}
          />
        </div>
      }
    />
  );
};

/**
 * Route every call through an assistant-ui-native rich renderer.
 *
 * The gated branches are chosen on the part's own `approval` field, before any
 * component that reads Redux is mounted. An ordinary tool call therefore never
 * subscribes to the store — and, less obviously, still renders on a surface
 * that has no store at all, which is how most of the tool-card tests mount it.
 */
export const ChatToolFallback: ToolCallMessagePartComponent = props => {
  if (props.toolName === 'task') return <SubagentCall {...props} />;
  if (!isApprovalPending(props.approval)) return <OpenHumanToolCall {...props} />;
  if (props.toolName === COMPOSIO_CONNECT_TOOL) return <ComposioConnectCall {...props} />;
  return <GatedToolCall {...props} />;
};

/** Keep the assistant-ui tool cards visible; each card owns its detail collapse. */
export const ChatToolGroup: FC<PropsWithChildren<{ group: ThreadGroupPart }>> = ({
  group,
  children,
}) => {
  const running = group.status.type === 'running';
  return (
    <ToolGroupRoot variant="ghost" defaultOpen>
      <ToolGroupTrigger count={group.indices.length} active={running} />
      <ToolGroupContent>{children}</ToolGroupContent>
    </ToolGroupRoot>
  );
};
