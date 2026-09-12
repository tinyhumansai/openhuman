import {
  CheckIcon,
  ChevronDownIcon,
  CircleXIcon,
  Loader2Icon,
  MessageCircleQuestionIcon,
  WorkflowIcon,
} from 'lucide-react';
import { useState } from 'react';

import { cn } from '../../../components/assistant-ui/lib/utils';
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '../../../components/assistant-ui/ui/collapsible';
import { Button } from '../../../components/ui';
import Badge from '../../../components/ui/Badge';
import WorktreeActions from '../../../components/worktree/WorktreeActions';
import { useT } from '../../../lib/i18n/I18nContext';
import {
  isActiveTimelineStatus,
  type SubagentActivity,
  type SubagentToolCallEntry,
  type SubagentTranscriptItem,
} from '../../../store/chatRuntimeSlice';
import { basename } from '../../../utils/pathUtils';
import { stripToolCallEnvelopes } from '../../../utils/toolTimelineFormatting';
import { BubbleMarkdown } from './AgentMessageBubble';
import { AssistantUiToolCallCard } from './AssistantUiToolCall';

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

function SubagentDetails({
  subagent,
  onView,
}: {
  subagent: SubagentActivity;
  onView?: () => void;
}) {
  const { t } = useT();
  const headerBits: string[] = [];
  if (subagent.mode) headerBits.push(subagent.mode);
  if (subagent.dedicatedThread) headerBits.push(t('conversations.toolTimeline.workerThread'));
  if (subagent.childIteration != null) {
    headerBits.push(
      subagent.childMaxIterations != null
        ? `${t('conversations.toolTimeline.turn')} ${subagent.childIteration}/${subagent.childMaxIterations}`
        : `${t('conversations.toolTimeline.step')} ${subagent.childIteration}`
    );
  } else if (subagent.iterations != null) {
    headerBits.push(
      subagent.iterations === 1
        ? `${subagent.iterations} ${t('chat.turn')}`
        : `${subagent.iterations} ${t('chat.turns')}`
    );
  }
  if (subagent.elapsedMs != null) {
    headerBits.push(
      subagent.elapsedMs >= 1000
        ? `${(subagent.elapsedMs / 1000).toFixed(1)}s`
        : `${subagent.elapsedMs}ms`
    );
  }
  const transcript = subagent.transcript ?? [];

  return (
    <div
      className="mt-1 space-y-0.5 text-[12px] text-content-muted"
      data-testid="subagent-activity">
      {headerBits.length > 0 ? (
        <div className="flex flex-wrap items-center gap-1.5">
          {headerBits.map(bit => (
            <Badge key={bit} className="rounded-full">
              {bit}
            </Badge>
          ))}
        </div>
      ) : null}
      {transcript.length > 0 ? (
        <div className="ml-1 space-y-0.5" data-testid="subagent-transcript">
          {transcript.map((item, index) =>
            item.kind === 'tool' ? (
              <ChildToolCallCard key={item.callId} call={item} />
            ) : (
              <Thought key={`thought-${index}`} text={item.text} />
            )
          )}
        </div>
      ) : subagent.toolCalls.length > 0 ? (
        <div className="ml-1 space-y-0.5">
          {subagent.toolCalls.map(call => (
            <ChildToolCallCard key={call.callId} call={call} />
          ))}
        </div>
      ) : null}
      {subagent.worktreePath ? (
        <div
          className="mt-1 space-y-1 rounded-md border border-line bg-surface-muted/70 p-1.5"
          data-testid="subagent-worktree">
          <div className="flex flex-wrap items-center gap-1.5">
            <span className="font-medium text-content-secondary">{t('worktree.label')}</span>
            <span
              className="truncate font-mono text-[12px] text-content-muted"
              title={subagent.worktreePath}>
              {basename(subagent.worktreePath)}
            </span>
            <Badge variant={subagent.isDirty ? 'warning' : 'success'} className="rounded-full">
              {subagent.isDirty ? t('worktree.dirty') : t('worktree.clean')}
            </Badge>
            {subagent.changedFiles?.length ? (
              <span className="text-[11px] text-content-faint">
                {subagent.changedFiles.length}{' '}
                {subagent.changedFiles.length === 1
                  ? t('worktree.changedFile')
                  : t('worktree.changedFiles')}
              </span>
            ) : null}
          </div>
          <WorktreeActions path={subagent.worktreePath} isDirty={subagent.isDirty} compact />
        </div>
      ) : null}
      {onView ? (
        <button
          type="button"
          onClick={onView}
          data-testid="subagent-view-processing"
          className="mt-0.5 rounded-full px-1.5 py-0.5 text-[12px] font-medium text-primary-600 hover:bg-primary-50 dark:text-primary-300 dark:hover:bg-primary-500/15">
          {t('conversations.subagent.viewProcessing')} →
        </button>
      ) : null}
    </div>
  );
}

/**
 * Statuses that mean the delegation is still in flight.
 *
 * `SubagentActivity.status` carries `running` | `awaiting_user` | `completed` |
 * `failed`, and collapsing that to a boolean is what produced two opposite
 * rendering bugs: a caller that omitted `running` showed a *failed* delegation
 * with a success check, while `status !== 'completed'` gave the same row an
 * endless spinner. Both call sites now ask this one question.
 *
 * The question itself is `isActiveTimelineStatus`, which the timeline row's
 * top-level `status` is also read through. This name survives because ~4 call
 * sites and their tests use it and it reads better beside a `SubagentActivity`
 * — but it must never grow a second opinion about what "active" means.
 */
export function isActiveSubagentStatus(status: string | undefined): boolean {
  return isActiveTimelineStatus(status);
}

/** Statuses that mean the delegation stopped without succeeding. */
function isFailedSubagentStatus(status: string | undefined): boolean {
  return status === 'failed' || status === 'cancelled';
}

/**
 * The delegation is parked on `ask_user_clarification` and cannot progress
 * until the user answers.
 *
 * Deliberately NOT a sub-case of {@link isActiveSubagentStatus}: both are true
 * at once and they answer different questions. "Active" decides whether the row
 * is still in flight (dashed border, no success check); "awaiting" decides
 * whether the blockage is *the user*. Folding the second into the first is what
 * rendered a child asking a question as an ordinary spinner labelled "running"
 * for as long as the gate stayed open.
 */
export function isAwaitingUserSubagentStatus(status: string | undefined): boolean {
  return status === 'awaiting_user';
}

/**
 * The child's question plus, when the host supplies `onAnswer`, a reply box.
 *
 * The answer is an ordinary user turn: the orchestrator is holding a
 * `[SUBAGENT_AWAITING_USER]` envelope that instructs it to relay the question
 * and resume with `continue_subagent` once the user responds
 * (`orchestration/tools/awaiting_user.rs`). So sending here goes through the
 * same composer path as typing the answer by hand, which is what makes the
 * queued-vs-new-turn decision in one place.
 *
 * `onAnswer` is optional because this card also renders on read-only,
 * historical surfaces (the process drawer, past-turn insights). Those pass no
 * handler and get the question without a dead reply box.
 */
function SubagentAwaitingUser({
  question,
  onAnswer,
}: {
  question?: string;
  onAnswer?: (text: string) => void;
}) {
  const { t } = useT();
  const [draft, setDraft] = useState('');
  // Local, optimistic: the core has no "answer received" event for this row.
  // It resumes by republishing `subagent_spawned`, which flips the status back
  // to running and unmounts this panel; until then the user needs to see that
  // their answer went somewhere.
  const [sent, setSent] = useState(false);

  const submit = () => {
    const text = draft.trim();
    if (!text || !onAnswer) return;
    onAnswer(text);
    setDraft('');
    setSent(true);
  };

  return (
    <div
      data-testid="subagent-awaiting-user"
      className="mt-1 space-y-1.5 rounded-lg border border-amber-300/70 bg-amber-50/70 p-2 dark:border-amber-400/30 dark:bg-amber-500/10">
      <p className="text-[12px] font-medium text-amber-800 dark:text-amber-200">
        {t('conversations.subagent.awaitingTitle')}
      </p>
      {question ? (
        // Plain text, not markdown: this is sub-agent-authored free text and
        // the card gains nothing from rendering links or images out of it.
        <p
          data-testid="subagent-awaiting-question"
          className="wrap-break-word whitespace-pre-wrap text-[12px] text-content-secondary">
          {question}
        </p>
      ) : null}
      {onAnswer ? (
        sent ? (
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
                // Enter sends, Shift+Enter is a newline. Matches the composer.
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
        )
      ) : null}
    </div>
  );
}

export function AssistantUiSubagentCall({
  activity,
  running,
  description,
  onView,
  onAnswer,
  defaultOpen = false,
}: {
  activity: SubagentActivity;
  running?: boolean;
  description?: string;
  onView?: () => void;
  /**
   * Send the user's reply to a delegation parked on `ask_user_clarification`.
   * Supplied only by the live chat surface; omit on read-only/historical
   * renders so no dead reply box appears.
   */
  onAnswer?: (text: string) => void;
  defaultOpen?: boolean;
}) {
  const { t } = useT();
  const name = activity.displayName ?? activity.agentId ?? 'subagent';
  // Default to the activity's own lifecycle rather than `false`: most call
  // sites pass no `running` prop at all, and treating every non-running
  // activity as finished-successfully is what rendered a failed delegation
  // with a success check.
  const active = running ?? isActiveSubagentStatus(activity.status);
  // Read from the activity, never from `running`: the assistant-ui surface
  // passes `running={result === undefined}`, which is `true` for a parked
  // delegation too, so a caller-supplied `running` cannot distinguish the two.
  const awaiting = isAwaitingUserSubagentStatus(activity.status);
  const failed = !active && isFailedSubagentStatus(activity.status);
  const [open, setOpen] = useState(defaultOpen);
  // A question the user cannot see is a question they cannot answer, and the
  // row is normally already mounted (and collapsed) as `running` by the time
  // the pause arrives, so `defaultOpen` is too late. Derived rather than an
  // effect that forces the state: the disclosure is pinned open only for as
  // long as the delegation is actually blocked on the user, and the user's own
  // open/closed choice is remembered underneath and restored on resume.
  const disclosureOpen = open || awaiting;
  return (
    <Collapsible
      open={disclosureOpen}
      onOpenChange={setOpen}
      data-slot="aui_subagent-call"
      data-testid="assistant-ui-subagent-call"
      data-status={activity.status ?? (active ? 'running' : 'completed')}
      className={cn(
        'aui-subagent-call border-border/60 dark:border-muted-foreground/15 rounded-xl border',
        active && 'border-dashed',
        awaiting && 'border-solid border-amber-300 dark:border-amber-400/40'
      )}>
      <CollapsibleTrigger className="group/subagent text-muted-foreground hover:text-foreground flex w-full items-center gap-2 px-3 py-2 text-sm transition-colors">
        <WorkflowIcon className="size-4 shrink-0" />
        <span className="text-start leading-none">
          Delegated to <b className="text-foreground">{name}</b>
        </span>
        {awaiting ? (
          // Not a spinner: the child is not working, it is blocked on the user.
          <span
            data-testid="subagent-awaiting-chip"
            className="flex shrink-0 items-center gap-1.5 rounded-full bg-amber-100 px-2 py-0.5 text-[11px] leading-none text-amber-800 dark:bg-amber-500/20 dark:text-amber-200">
            <MessageCircleQuestionIcon className="size-3" />
            {t('conversations.subagent.statusAwaitingUser')}
          </span>
        ) : active ? (
          <span className="bg-muted text-muted-foreground flex shrink-0 items-center gap-1.5 rounded-full px-2 py-0.5 text-[11px] leading-none">
            <Loader2Icon className="size-3 animate-spin [animation-duration:0.6s]" /> running
          </span>
        ) : (
          <span className="text-muted-foreground flex shrink-0 items-center gap-1.5 text-[11px] leading-none">
            {failed ? <CircleXIcon className="size-3.5" /> : <CheckIcon className="size-3.5" />}
            {failed ? <span>{activity.status}</span> : null}
            {activity.elapsedMs != null ? (
              <span className="tabular-nums">{(activity.elapsedMs / 1000).toFixed(1)}s</span>
            ) : null}
          </span>
        )}
        <ChevronDownIcon className="ml-auto size-4 shrink-0 -rotate-90 transition-transform group-data-[state=open]/subagent:rotate-0" />
      </CollapsibleTrigger>
      <CollapsibleContent className="px-3 pb-3">
        {description ? <p className="text-muted-foreground text-xs">{description}</p> : null}
        {awaiting ? (
          <SubagentAwaitingUser question={activity.awaitingQuestion} onAnswer={onAnswer} />
        ) : null}
        <SubagentDetails subagent={activity} onView={onView} />
      </CollapsibleContent>
    </Collapsible>
  );
}
