'use client';

/**
 * Renders a bare {@link SubagentActivity} (not wrapped in an assistant-ui
 * message part) through the vendored `elements/task-card.tsx` shell — the
 * same `TaskCard` `SubagentTaskCard.tsx` uses for a live `task` tool-call
 * part.
 *
 * `SubagentTaskCard` cannot be reused directly here: it is a
 * `ToolCallMessagePartComponent` that reads `args`/`result`/`messages` off an
 * assistant-ui part, and it wires the awaiting-user reply box through
 * `useAui()` — which requires an ambient `AssistantRuntimeProvider`. This
 * component's callers (`ToolTimelineAdapter`, `AgentProcessSourcePanel`) can
 * render outside that provider (e.g. `TranscriptOverlays` is a sibling of
 * `AssistantUiChat`, not a descendant of it), so this stays read-only: the
 * awaiting-user question is shown as text with no reply box.
 *
 * The nested transcript does NOT go through the vendored `TaskTranscript`
 * (`elements/task-card.aui.tsx`) `SubagentTaskCard` uses for its live
 * delegation: `TaskTranscript`'s `NestedMessage` renders
 * `MessagePrimitive.Root`, which unconditionally calls
 * `useThreadViewportStore()` — satisfied only by an ambient
 * `ThreadPrimitive.Viewport`, which itself requires a real
 * `AssistantRuntimeProvider` (`useAuiState` inside its top-anchor tracking).
 * None of this component's callers render inside one, so reusing
 * `TaskTranscript` here throws `This component must be used within
 * ThreadPrimitive.Viewport.` the moment the disclosure opens. Instead the
 * nested activity renders directly off the `SubagentActivity` fields — the
 * same data `TaskTranscript` would have been fed via
 * `providers/assistantUiMessages.ts#subagentMessages`, just rendered by
 * `AssistantUiToolCallCard` (already standalone-safe: it takes its tool-call
 * shape as plain props) and `BubbleMarkdown` instead of assistant-ui's
 * message primitives.
 */
import { TaskCard, type TaskCardState } from '../../../components/assistant-ui/elements/task-card';
import { formatElapsed } from '../../../components/assistant-ui/utils/task';
import { useT } from '../../../lib/i18n/I18nContext';
import {
  isActiveTimelineStatus,
  type SubagentActivity,
  type SubagentToolCallEntry,
  type SubagentTranscriptItem,
} from '../../../store/chatRuntimeSlice';
import { stripToolCallEnvelopes } from '../../../utils/toolTimelineFormatting';
import { BubbleMarkdown } from '../components/AgentMessageBubble';
import { AssistantUiToolCallCard } from '../components/AssistantUiToolCall';

type ChildToolCall = SubagentToolCallEntry | Extract<SubagentTranscriptItem, { kind: 'tool' }>;

function ChildToolCallCard({ call }: { call: ChildToolCall }) {
  return (
    <AssistantUiToolCallCard
      toolName={call.toolName}
      args={call.args}
      result={call.result}
      status={call.status}
      displayName={call.displayName}
      detail={call.detail}
      elapsedMs={call.elapsedMs}
      failure={call.failure}
    />
  );
}

function Thought({ text }: { text: string }) {
  const clean = stripToolCallEnvelopes(text).trim();
  if (!clean) return null;
  return (
    <div
      data-testid="subagent-thought"
      className="my-0.5 wrap-break-word [&_.prose]:text-[12px] [&_.prose]:leading-relaxed [&_.prose]:text-content-muted [&_.prose_strong]:text-content-muted [&_.prose_:is(h1,h2,h3,h4,h5,h6)]:text-[12px] [&_.prose_:is(h1,h2,h3,h4,h5,h6)]:text-content-muted">
      <BubbleMarkdown content={clean} />
    </div>
  );
}

function stateOf(activity: SubagentActivity): TaskCardState {
  if (activity.status === 'awaiting_user') return 'waiting';
  if (isActiveTimelineStatus(activity.status)) return 'working';
  if (activity.status === 'failed') return 'failed';
  if (activity.status === 'cancelled') return 'cancelled';
  if (activity.status === 'incomplete') return 'incomplete';
  return 'done';
}

/** The nested activity: the child's interleaved transcript, or its flat `toolCalls` list. */
function ActivityTranscript({ activity }: { activity: SubagentActivity }) {
  const transcript = activity.transcript ?? [];
  if (transcript.length > 0) {
    return (
      <div className="space-y-0.5" data-testid="subagent-transcript">
        {transcript.map((item, index) =>
          item.kind === 'tool' ? (
            <ChildToolCallCard key={item.callId} call={item} />
          ) : (
            <Thought key={`thought-${index}`} text={item.text} />
          )
        )}
      </div>
    );
  }
  if (activity.toolCalls.length > 0) {
    return (
      <div className="space-y-0.5">
        {activity.toolCalls.map(call => (
          <ChildToolCallCard key={call.callId} call={call} />
        ))}
      </div>
    );
  }
  return null;
}

export function SubagentActivityCard({ activity }: { activity: SubagentActivity }) {
  const { t } = useT();
  const state = stateOf(activity);
  const name = activity.displayName ?? activity.agentId ?? 'subagent';
  const elapsed = activity.elapsedMs !== undefined ? formatElapsed(activity.elapsedMs) : undefined;
  const awaiting = state === 'waiting';
  const incomplete = state === 'incomplete';
  const hasTranscript = (activity.transcript?.length ?? 0) > 0 || activity.toolCalls.length > 0;

  const actions =
    awaiting || incomplete ? (
      <div className="flex flex-col gap-2.5">
        {incomplete ? (
          <p
            role="status"
            data-testid="subagent-incomplete"
            className="text-[12px] font-medium text-amber-800 dark:text-amber-200">
            {t('conversations.subagent.incompleteTitle')}
          </p>
        ) : null}
        {awaiting ? (
          <div data-testid="subagent-awaiting-user" className="flex flex-col gap-1.5">
            <p className="text-[12px] font-medium text-amber-800 dark:text-amber-200">
              {t('conversations.subagent.awaitingTitle')}
            </p>
            {activity.awaitingQuestion ? (
              <p
                data-testid="subagent-awaiting-question"
                className="wrap-break-word whitespace-pre-wrap text-[12px] text-content-secondary">
                {activity.awaitingQuestion}
              </p>
            ) : null}
          </div>
        ) : null}
      </div>
    ) : undefined;

  const resultNode =
    activity.output && (state === 'done' || state === 'failed' || state === 'incomplete') ? (
      <p className="m-0 whitespace-pre-wrap">{activity.output}</p>
    ) : undefined;

  return (
    <TaskCard
      data-testid="assistant-ui-subagent-call"
      data-status={activity.status ?? state}
      label={`${t('conversations.tools.delegatedTo').replace('{agent}', name)}`}
      meta={activity.mode}
      state={state}
      elapsed={elapsed}
      actions={actions}
      result={resultNode}>
      {hasTranscript ? (
        <div data-testid="subagent-activity">
          <ActivityTranscript activity={activity} />
        </div>
      ) : undefined}
    </TaskCard>
  );
}

export default SubagentActivityCard;
