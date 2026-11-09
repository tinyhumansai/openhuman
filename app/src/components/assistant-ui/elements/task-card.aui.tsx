'use client';

/**
 * Vendored from the assistant-ui `task-card` registry item's `.aui` wiring
 * (https://r.assistant-ui.com/styles/base-nova/task-card.json,
 * `elements/task-card.aui.tsx` upstream). This is the generic renderer
 * assistant-ui's `MessagePrimitive.GroupedParts` falls back to for ANY
 * tool-call part that carries nested `messages` and has no toolkit entry of
 * its own; OpenHuman's own `task` toolkit entry
 * (`features/conversations/aui/toolkit.tsx`) renders sub-agent delegations
 * through `features/conversations/aui/SubagentTaskCard.tsx` instead, which
 * needs OpenHuman-specific affordances (the awaiting-user reply box) this
 * generic card has no slot for.
 *
 * Changes from upstream:
 * - `cn` import path (`@/components/assistant-ui/lib/utils`).
 * - `@/components/assistant-ui/elements/markdown-text` ->
 *   `@/components/assistant-ui/markdown-text` (this app's actual path; there
 *   is no separate `elements/markdown-text.tsx`).
 * - `./tool-fallback.aui` -> `./tool-fallback` (this app vendored the
 *   `tool-fallback` registry item's `.aui` content directly under that
 *   filename, without a plain/`.aui` split).
 * - The hard-coded role tag (`instruction`/`agent`/`system`) and `TaskGroup`
 *   summary/"Show N more" copy are now `roleLabels`/`strings` props (English
 *   defaults matching upstream) so a host can supply `useT()`-sourced copy —
 *   see `SubagentTaskCard.tsx`'s `TaskTranscript` call.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import { MarkdownText } from '@/components/assistant-ui/markdown-text';
import {
  MessagePrimitive,
  ReadonlyThreadProvider,
  type ThreadMessage,
  ThreadPrimitive,
  type ToolCallMessagePart,
  type ToolCallMessagePartComponent,
  type ToolCallMessagePartProps,
  type ToolCallMessagePartStatus,
  useAuiState,
} from '@assistant-ui/react';
import { type FC } from 'react';

import { formatElapsed, taskLabel, taskMeta, taskStateOf, useTaskElapsed } from '../utils/task';
import { mono } from './surfaces';
import { TaskCard as TaskCardBase } from './task-card';
import {
  formatUnknownValue,
  offersInterruptAction,
  ToolFallback,
  ToolFallbackApproval,
  ToolFallbackError,
} from './tool-fallback';

export type { TaskCardState } from './task-card';

export type TaskPart = ToolCallMessagePart & {
  readonly status: ToolCallMessagePartStatus;
} & Partial<Pick<ToolCallMessagePartProps, 'addResult' | 'resume' | 'respondToApproval'>>;

const isTaskPart = (part: { readonly type: string; readonly messages?: unknown }) =>
  part.type === 'tool-call' && part.messages !== undefined;

/** English defaults for a nested transcript message's role tag; override via `TaskTranscript`'s `roleLabels` prop. */
export interface TaskTranscriptRoleLabels {
  user: string;
  assistant: string;
  system: string;
}

const DEFAULT_ROLE_LABELS: TaskTranscriptRoleLabels = {
  user: 'instruction',
  assistant: 'agent',
  system: 'system',
};

// A transcript is a readonly snapshot, so a call waiting inside it is answered where its run is live, and renders here as paused on something else.
const NestedToolCall: ToolCallMessagePartComponent = ({ approval, interrupt, ...rest }) => {
  const part =
    rest.status.type === 'requires-action'
      ? { ...rest, status: { type: 'requires-action', reason: 'interrupt' } as const }
      : rest;
  return isTaskPart(part) ? <TaskCard part={part} /> : <ToolFallback {...part} />;
};

const NestedMessage: FC<{ roleLabels: TaskTranscriptRoleLabels }> = ({ roleLabels }) => {
  const role = useAuiState(s => s.message.role);

  return (
    <MessagePrimitive.Root
      data-slot="aui_task-transcript-message"
      data-role={role}
      className="flex flex-col gap-1 text-xs leading-relaxed">
      <span className={cn(mono, 'text-muted-foreground')}>{roleLabels[role]}</span>
      <MessagePrimitive.Parts
        components={{ Text: MarkdownText, tools: { Fallback: NestedToolCall } }}
      />
    </MessagePrimitive.Root>
  );
};

export const TaskTranscript: FC<{
  messages: readonly ThreadMessage[];
  roleLabels?: TaskTranscriptRoleLabels;
}> = ({ messages, roleLabels = DEFAULT_ROLE_LABELS }) => (
  <ReadonlyThreadProvider messages={messages}>
    <ThreadPrimitive.Messages>
      {() => <NestedMessage roleLabels={roleLabels} />}
    </ThreadPrimitive.Messages>
  </ReadonlyThreadProvider>
);

const TaskResult: FC<{ result: unknown }> = ({ result }) =>
  typeof result === 'string' ? (
    <p className="m-0 whitespace-pre-wrap">{result}</p>
  ) : (
    <pre className="m-0 overflow-x-auto whitespace-pre-wrap">{formatUnknownValue(result, 2)}</pre>
  );

const TaskCard: FC<{ part: TaskPart; className?: string }> = ({ part, className }) => {
  const elapsedMs = useTaskElapsed(
    part.timing,
    part.status.type === 'running' || part.status.type === 'requires-action'
  );
  const messages = part.messages ?? [];
  const showError =
    part.status.type === 'incomplete' &&
    part.status.error !== undefined &&
    part.status.error !== null;
  const result =
    showError || part.result !== undefined ? (
      <>
        {showError && <ToolFallbackError status={part.status} />}
        {part.result !== undefined && <TaskResult result={part.result} />}
      </>
    ) : undefined;
  const approvalPending =
    part.approval == null ||
    (part.approval.approved === undefined && part.approval.resolution === undefined);
  const actions =
    part.status.type === 'requires-action' &&
    approvalPending &&
    offersInterruptAction(part.status, part.approval, part.interrupt) ? (
      <ToolFallbackApproval
        status={part.status}
        {...(part.approval !== undefined && { approval: part.approval })}
        {...(part.interrupt !== undefined && { interrupt: part.interrupt })}
        {...(part.addResult && { addResult: part.addResult })}
        {...(part.resume && { resume: part.resume })}
        {...(part.respondToApproval && { respondToApproval: part.respondToApproval })}
      />
    ) : undefined;

  return (
    <TaskCardBase
      className={className}
      label={taskLabel(part.toolName, part.args)}
      meta={taskMeta(part.args)}
      state={taskStateOf(part.status, part.isError)}
      elapsed={elapsedMs === undefined ? undefined : formatElapsed(elapsedMs)}
      actions={actions}
      result={result}>
      {messages.length > 0 ? <TaskTranscript messages={messages} /> : undefined}
    </TaskCardBase>
  );
};
