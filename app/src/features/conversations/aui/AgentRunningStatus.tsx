'use client';

/**
 * The `Thread`'s `RunningStatus` slot (`components/assistant-ui/thread.tsx`),
 * on the assistant-ui surface. Replaces `AssistantUiInferenceStatus.tsx` /
 * `aui/InferenceStatusLine.tsx` (deleted).
 *
 * Where those read `chatRuntime.inferenceStatusByThread` (phase/active tool/
 * active subagent) through the runtime's `extras` channel, this reads
 * assistant-ui's own `s.thread.tasks` (`elements/agent-status.aui.tsx`'s
 * `TaskTray`) — the delegations the `task` toolkit entry registered as nested
 * tasks (`providers/assistantUiMessages.ts`'s `subagentMessages`). A plain
 * tool call (read a file, run a shell command, web search) is not a "task" by
 * that definition — it has no nested transcript — so it never reaches this
 * component at all; assistant-ui's own running-message indicator already
 * signals "something is happening" for those, same as before.
 *
 * Three states, highest priority first (after OpenClaw's working row):
 *
 * 1. **Waiting on you.** A parked tool approval, a plan review gate, or an
 *    unanswered top-level `ask_user_clarification` means the run cannot move
 *    until the user acts. Saying "Thinking…" there is wrong, so the slot shows
 *    a still (non-shimmering) attention line naming what it waits for.
 * 2. **Waiting on sub-agents.** While delegations run, the `TaskTray`, led by
 *    "Waiting on {name}" / "Waiting on {count} subagents" when the parent
 *    itself is parked on them (`inferenceStatusByThread.phase === 'subagent'`).
 * 3. **Thinking.** The vendored `GenerationLoader`, with "Thinking · {elapsed}"
 *    once a second has passed. The clock starts when this slot mounts, which is
 *    when the turn starts running: the runtime keeps no turn-start timestamp.
 *    While the turn summarizes earlier conversation before its next model call
 *    (`inferenceStatusByThread.phase === 'compacting'`), the same loader says
 *    so instead: that can take minutes with nothing else to show.
 */
import { useAuiState } from '@assistant-ui/react';
import { useEffect, useState } from 'react';

import {
  TaskTray,
  useTaskSummary,
} from '../../../components/assistant-ui/elements/agent-status.aui';
import { GenerationLoader } from '../../../components/assistant-ui/elements/loading-state';
import { formatElapsed, useTaskElapsed } from '../../../components/assistant-ui/utils/task';
import { useT } from '../../../lib/i18n/I18nContext';
import { useAuiThreadId } from '../../../providers/AssistantUiRuntimeProvider';
import { useAppSelector } from '../../../store/hooks';

/** English defaults mapped onto `AgentStatusStrings` via `useT()`. */
function useAgentStatusStrings() {
  const { t } = useT();
  return {
    taskOne: t('conversations.tasks.taskOne'),
    taskOther: t('conversations.tasks.taskOther'),
    running: t('conversations.tasks.running'),
    waitingForInput: t('conversations.tasks.waitingForInput'),
    done: t('conversations.tasks.done'),
    failed: t('conversations.tasks.failed'),
    of: t('conversations.tasks.of'),
  };
}

/** What the run is blocked on, if it is blocked on the user. */
export type WaitingOnUser = 'approval' | 'review' | 'answer';

/** The top-level clarification tool (`ChatToolParts.tsx`'s `ElicitationCall`). */
const ASK_USER_CLARIFICATION_TOOL = 'ask_user_clarification';

interface MessageLike {
  readonly role: string;
  readonly content: ReadonlyArray<{
    readonly type: string;
    readonly toolName?: string;
    readonly result?: unknown;
  }>;
}

/** Whether the newest assistant message carries an unanswered clarification. */
export function hasPendingClarification(messages: ReadonlyArray<MessageLike>): boolean {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message.role !== 'assistant') continue;
    return message.content.some(
      part =>
        part.type === 'tool-call' &&
        part.toolName === ASK_USER_CLARIFICATION_TOOL &&
        part.result === undefined
    );
  }
  return false;
}

/**
 * Whether the running turn has delegated work of its own.
 *
 * assistant-ui's `thread.tasks` collects delegations from every message in the
 * thread, so a chat that ran a subagent once would hide "Thinking…" on every
 * later turn. The running turn is the newest message when it is the
 * assistant's; a turn whose reply has not appeared yet has no tasks.
 */
export function turnHasTasks(
  tasks: ReadonlyArray<{ readonly messageId: string }>,
  messages: ReadonlyArray<{ readonly id: string; readonly role: string }>
): boolean {
  const newest = messages.at(-1);
  if (newest?.role !== 'assistant') return false;
  return tasks.some(task => task.messageId === newest.id);
}

/** Approval outranks review outranks answer: the most blocking gate first. */
export function waitingOnUser(args: {
  approval: boolean;
  review: boolean;
  answer: boolean;
}): WaitingOnUser | null {
  if (args.approval) return 'approval';
  if (args.review) return 'review';
  if (args.answer) return 'answer';
  return null;
}

const WAITING_KEYS: Record<WaitingOnUser, string> = {
  approval: 'chat.status.waitingApproval',
  review: 'chat.status.waitingReview',
  answer: 'chat.status.waitingAnswer',
};

function useWaitingOnUser(): WaitingOnUser | null {
  const threadId = useAuiThreadId();
  const approval = useAppSelector(state =>
    threadId ? Boolean(state.chatRuntime.pendingApprovalByThread?.[threadId]) : false
  );
  const review = useAppSelector(state =>
    threadId ? Boolean(state.chatRuntime.pendingPlanReviewByThread?.[threadId]) : false
  );
  const answer = useAuiState(s => hasPendingClarification(s.thread.messages));
  return waitingOnUser({ approval, review, answer });
}

/** Whether the parent turn is itself parked waiting on its delegations. */
function useParentWaitsOnSubagents(): boolean {
  const threadId = useAuiThreadId();
  return useAppSelector(state =>
    threadId ? state.chatRuntime.inferenceStatusByThread?.[threadId]?.phase === 'subagent' : false
  );
}

/** Whether the turn is summarizing earlier conversation before its next call. */
function useCompacting(): boolean {
  const threadId = useAuiThreadId();
  return useAppSelector(state =>
    threadId ? state.chatRuntime.inferenceStatusByThread?.[threadId]?.phase === 'compacting' : false
  );
}

function WaitingLine({ kind }: { kind: WaitingOnUser }) {
  const { t } = useT();
  return (
    <div
      data-testid="agent-running-status-waiting"
      data-waiting={kind}
      role="status"
      className="flex items-center gap-2 px-2 text-sm font-medium text-amber-600 dark:text-amber-400">
      <span aria-hidden className="relative flex size-2">
        <span className="absolute inline-flex size-full rounded-full bg-amber-500/40 motion-safe:animate-ping" />
        <span className="relative inline-flex size-2 rounded-full bg-amber-500" />
      </span>
      {t(WAITING_KEYS[kind])}
    </div>
  );
}

export function AgentRunningStatus() {
  const { t } = useT();
  const summary = useTaskSummary();
  const strings = useAgentStatusStrings();
  const waiting = useWaitingOnUser();
  const parentWaits = useParentWaitsOnSubagents();
  const compacting = useCompacting();
  const [tick, setTick] = useState(0);
  const [startedAt] = useState(() => Date.now());
  const turnTasks = useAuiState(s => turnHasTasks(s.thread.tasks, s.thread.messages));
  const thinking = !turnTasks && waiting === null;
  const elapsedMs = useTaskElapsed({ startedAt }, thinking);

  useEffect(() => {
    if (!thinking) return undefined;
    const timer = window.setInterval(() => setTick(value => value + 1), 120);
    return () => window.clearInterval(timer);
  }, [thinking]);

  if (waiting !== null) return <WaitingLine kind={waiting} />;

  if (!turnTasks) {
    const elapsed =
      elapsedMs !== undefined && elapsedMs >= 1000 ? formatElapsed(elapsedMs) : undefined;
    const label = compacting
      ? elapsed
        ? t('chat.status.compactingElapsed').replace('{elapsed}', elapsed)
        : t('chat.status.compacting')
      : elapsed
        ? t('chat.status.thinkingElapsed').replace('{elapsed}', elapsed)
        : t('chat.thinkingDots');
    return (
      <GenerationLoader
        data-testid="agent-running-status-thinking"
        label={label}
        tick={tick}
        variant="rounded"
        className="flex-row justify-start gap-2.5 px-2 [&>div]:gap-0.5 [&>div>span]:size-1"
      />
    );
  }

  const waitingOnSubagents =
    parentWaits && summary.running > 0
      ? summary.running === 1 && summary.runningLabel
        ? t('chat.status.waitingOnSubagent').replace('{name}', summary.runningLabel)
        : t('chat.status.waitingOnSubagents').replace('{count}', String(summary.running))
      : null;

  return (
    <div className="flex items-center gap-2">
      {waitingOnSubagents && (
        <span
          data-testid="agent-running-status-subagents"
          className="px-2 text-sm text-content-muted">
          {waitingOnSubagents}
        </span>
      )}
      <TaskTray data-testid="agent-running-status-tasks" strings={strings} />
    </div>
  );
}

export default AgentRunningStatus;
