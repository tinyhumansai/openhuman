'use client';

/**
 * OpenHuman glue over the vendored `elements/task-card.tsx` for a sub-agent
 * delegation (`task` toolkit entry — `features/conversations/aui/toolkit.tsx`).
 *
 * Replaces `AssistantUiSubagentCall.tsx` (deleted). The delegation's full
 * transcript is no longer read through a host-supplied "View full processing"
 * button into `SubagentDrawer` — assistant-ui's own `messages` part field
 * (`providers/assistantUiMessages.ts`'s `subagentMessages`) carries it, and
 * `TaskCardBase`'s built-in disclosure renders it inline as the nested
 * transcript.
 *
 * Not the vendored `elements/task-card.aui.tsx`'s own `TaskCard` wrapper:
 * that one derives its `actions` purely from `part.approval`/`part.interrupt`,
 * which has no slot for the two OpenHuman-specific actions a delegation
 * needs — the awaiting-user reply box and the worktree open/diff/remove row
 * — so this adapter is built directly from the raw `TaskCard`
 * (`elements/task-card.tsx`) + `utils/task.ts` pieces instead, with those
 * actions supplied explicitly.
 */
import { type ToolCallMessagePartComponent, useAui } from '@assistant-ui/react';
import { useCallback, useState } from 'react';

import { TaskCard, type TaskCardState } from '../../../components/assistant-ui/elements/task-card';
import { TaskTranscript } from '../../../components/assistant-ui/elements/task-card.aui';
import { useDisclosure } from '../../../components/assistant-ui/lib/useDisclosure';
import { formatElapsed } from '../../../components/assistant-ui/utils/task';
import { Button } from '../../../components/ui';
import Badge from '../../../components/ui/Badge';
import WorktreeActions from '../../../components/worktree/WorktreeActions';
import { useT } from '../../../lib/i18n/I18nContext';
import { subagentMessages } from '../../../providers/assistantUiMessages';
import { subagentApi } from '../../../services/api/subagentApi';
import { type SubagentActivity, subagentCancelResolved } from '../../../store/chatRuntimeSlice';
import { useAppDispatch } from '../../../store/hooks';
import { basename } from '../../../utils/pathUtils';

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

/**
 * `providers/assistantUiMessages.ts`'s `toolPart` puts the live activity on
 * `args.progress` while the delegation is running (mirroring `entry.subagent`)
 * and the settled `{status, activity}` envelope on `result` once it is not —
 * see that module's `toolPart`. Either shape yields the same
 * {@link SubagentActivity}; only the outer row status differs in reliability
 * (settled: the real {@link import('../../../store/chatRuntimeSlice').ToolTimelineEntryStatus};
 * running: derived from the activity's own `status` field).
 */
function readSubagentCall(
  args: unknown,
  result: unknown
): { activity: SubagentActivity | undefined; state: TaskCardState } {
  if (result && typeof result === 'object' && 'activity' in (result as Record<string, unknown>)) {
    const envelope = result as { status?: string; activity?: unknown };
    const activity = asSubagentActivity(envelope.activity);
    const status = envelope.status;
    const state: TaskCardState =
      status === 'error' ? 'failed' : status === 'cancelled' ? 'cancelled' : 'done';
    return { activity, state };
  }
  const progress =
    args && typeof args === 'object'
      ? asSubagentActivity((args as { progress?: unknown }).progress)
      : undefined;
  return {
    activity: progress,
    state: progress?.status === 'awaiting_user' ? 'waiting' : 'working',
  };
}

/** The child's question plus a reply box, sent via `aui.thread.append` — an ordinary new user turn. */
function AwaitingUserActions({ activity }: { activity: SubagentActivity }) {
  const { t } = useT();
  const aui = useAui();
  const [draft, setDraft] = useState('');
  const [sent, setSent] = useState(false);

  const submit = useCallback(() => {
    const text = draft.trim();
    if (!text) return;
    void aui.thread.append({ role: 'user', content: [{ type: 'text', text }] });
    setDraft('');
    setSent(true);
  }, [aui, draft]);

  return (
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
      {sent ? (
        <p className="text-[11px] text-content-muted" data-testid="subagent-answer-sent">
          {t('conversations.subagent.answerSent')}
        </p>
      ) : (
        <div className="flex items-end gap-1.5">
          <textarea
            rows={1}
            value={draft}
            data-testid="subagent-answer-input"
            aria-label={t('conversations.subagent.answerPlaceholder')}
            placeholder={t('conversations.subagent.answerPlaceholder')}
            onChange={event => setDraft(event.target.value)}
            onKeyDown={event => {
              if (event.key === 'Enter' && !event.shiftKey) {
                event.preventDefault();
                submit();
              }
            }}
            className="min-h-[28px] flex-1 resize-y rounded-md border border-line bg-surface px-2 py-1 text-[12px] text-content outline-none focus:border-primary-500"
          />
          <Button
            type="button"
            size="xs"
            variant="primary"
            analyticsId="subagent-answer-send"
            data-testid="subagent-answer-send"
            disabled={draft.trim().length === 0}
            onClick={submit}>
            {t('conversations.subagent.answerSend')}
          </Button>
        </div>
      )}
    </div>
  );
}

/**
 * The old drawer's "Cancel task" button, ported here as a `TaskCard` action:
 * aborts a still-running (or awaiting-user) detached sub-agent via
 * `openhuman.subagent_cancel`. Hidden once the delegation has settled
 * (`done`/`failed`/`cancelled`) — there is nothing left to abort.
 */
function CancelTaskAction({ taskId }: { taskId: string }) {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const [cancelling, setCancelling] = useState(false);
  const [failed, setFailed] = useState(false);

  const onCancel = useCallback(() => {
    setFailed(false);
    setCancelling(true);
    void subagentApi
      .cancel(taskId)
      // The core's answer is the only signal this card gets for a run it just
      // aborted or that already ended, so settle on it rather than waiting for
      // a `subagent_failed` that may never come.
      .then(({ cancelled, outcome }) => {
        dispatch(subagentCancelResolved({ taskId, cancelled, outcome }));
      })
      .catch(() => {
        setFailed(true);
      })
      .finally(() => {
        setCancelling(false);
      });
  }, [dispatch, taskId]);

  return (
    <div className="flex flex-col gap-1">
      <Button
        type="button"
        size="xs"
        variant="secondary"
        analyticsId="subagent-cancel-task"
        data-testid="subagent-cancel-task"
        disabled={cancelling}
        onClick={onCancel}>
        {cancelling ? t('conversations.subagent.cancelling') : t('conversations.subagent.cancel')}
      </Button>
      {failed ? (
        <p className="text-[11px] text-red-600 dark:text-red-400">
          {t('conversations.subagent.cancelFailed')}
        </p>
      ) : null}
    </div>
  );
}

function WorktreeRow({ activity }: { activity: SubagentActivity }) {
  const { t } = useT();
  if (!activity.worktreePath) return null;
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="font-medium text-content-secondary">{t('worktree.label')}</span>
        <span
          className="truncate font-mono text-[12px] text-content-muted"
          title={activity.worktreePath}>
          {basename(activity.worktreePath)}
        </span>
        <Badge variant={activity.isDirty ? 'warning' : 'success'} className="rounded-full">
          {activity.isDirty ? t('worktree.dirty') : t('worktree.clean')}
        </Badge>
      </div>
      <WorktreeActions path={activity.worktreePath} isDirty={activity.isDirty} compact />
    </div>
  );
}

/** Adapt an assistant-ui `task` part onto {@link TaskCard} for a sub-agent delegation. */
export const SubagentTaskCard: ToolCallMessagePartComponent = ({
  args,
  result,
  messages,
  toolCallId,
}) => {
  const { t } = useT();
  const { activity, state } = readSubagentCall(args, result);
  const fallbackAgent = (args as { subagent_type?: string } | undefined)?.subagent_type;
  const resolved = activity ?? {
    taskId: 'pending-subagent',
    agentId: fallbackAgent ?? 'subagent',
    toolCalls: [],
  };
  const name = resolved.displayName ?? resolved.agentId ?? 'subagent';
  const elapsed = resolved.elapsedMs !== undefined ? formatElapsed(resolved.elapsedMs) : undefined;
  const awaiting = state === 'waiting' && resolved.status === 'awaiting_user';
  // The user's own open/closed choice, remembered by the part's tool-call id
  // so it survives assistant-ui's remounts (a virtualized history, a thread
  // switch, a part whose shape changes) — see `useDisclosure`. Pinned open
  // for as long as the delegation is actually awaiting a reply, regardless of
  // that choice: a question the user cannot see is a question they cannot
  // answer, and the row is normally already mounted (and collapsed) by the
  // time the pause arrives, so a one-time `defaultOpen` would be too late.
  const [open, setOpen] = useDisclosure(toolCallId ? `subagent:${toolCallId}` : undefined, false);
  const disclosureOpen = open || awaiting;

  const cancellable =
    (state === 'working' || state === 'waiting') && resolved.taskId !== 'pending-subagent';

  const actions =
    awaiting || resolved.worktreePath || cancellable ? (
      <div className="flex flex-col gap-2.5">
        {awaiting ? <AwaitingUserActions activity={resolved} /> : null}
        <WorktreeRow activity={resolved} />
        {cancellable ? <CancelTaskAction taskId={resolved.taskId} /> : null}
      </div>
    ) : undefined;

  const resultNode =
    resolved.output && (state === 'done' || state === 'failed') ? (
      <p className="m-0 whitespace-pre-wrap">{resolved.output}</p>
    ) : undefined;

  // `messages` (the part's own nested transcript, built by `subagentMessages`)
  // is preferred over recomputing it here — it is the SAME data, but reusing
  // the part's own field keeps this renderer correct for a replayed/settled
  // part too, where `entry.subagent` is no longer live Redux state.
  const nestedMessages = messages ?? subagentMessages(resolved);

  return (
    <TaskCard
      data-testid="assistant-ui-subagent-call"
      data-status={resolved.status ?? state}
      label={`${t('conversations.tools.delegatedTo').replace('{agent}', name)}`}
      meta={resolved.mode}
      state={state}
      elapsed={elapsed}
      open={disclosureOpen}
      onOpenChange={setOpen}
      actions={actions}
      result={resultNode}>
      {nestedMessages.length > 0 ? (
        <div data-testid="subagent-activity">
          <TaskTranscript
            messages={nestedMessages}
            roleLabels={{
              user: t('conversations.subagent.transcript.instruction'),
              assistant: t('conversations.subagent.transcript.agent'),
              system: t('conversations.subagent.transcript.system'),
            }}
          />
        </div>
      ) : undefined}
    </TaskCard>
  );
};

export default SubagentTaskCard;
