'use client';

/**
 * Not a vendored element. A local composition of two vendored primitives —
 * `ToolGroupRoot` / `ToolGroupTrigger` / `ToolGroupContent`
 * (`elements/tool-group.tsx`, from the assistant-ui `tool-group` registry
 * item) and `OpenHumanReasoningGroup` (`reasoning-group.tsx`, over the
 * `reasoning-trace` element) — that upstream has no equivalent for: one
 * disclosure per `MessagePrimitive.GroupedParts` run of reasoning-and-tool
 * activity, rather than a separate collapsible per part type. See the
 * `ActivityGroup` doc comment below for why.
 */
import {
  ToolGroupContent,
  ToolGroupRoot,
  ToolGroupTrigger,
} from '@/components/assistant-ui/elements/tool-group';
import { OpenHumanReasoningGroup } from '@/components/assistant-ui/reasoning-group';
import { formatElapsed } from '@/components/assistant-ui/utils/task';
import { useT } from '@/lib/i18n/I18nContext';
import { type MessagePrimitive, useAuiState } from '@assistant-ui/react';
import { type FC, type PropsWithChildren, useState } from 'react';

export type ActivityGroupPart = MessagePrimitive.GroupedParts.GroupPart;

type Translate = (key: string) => string;

/**
 * Trigger text for a run of reasoning and tool calls.
 *
 * Tool calls are what the reader counts; reasoning is either there or not, so
 * it is named rather than counted. The group only exists when it holds at
 * least one of the two, so the empty fallback is never shown in practice.
 *
 * `workedMs`, when known, turns a settled group into OpenClaw's turn receipt:
 * "Worked for 42s · 3 tool calls".
 */
export function activityGroupLabel(
  reasoningCount: number,
  toolCount: number,
  t: Translate,
  workedMs?: number
): string {
  const tools =
    toolCount > 0
      ? t(toolCount === 1 ? 'chat.tools.callOne' : 'chat.tools.callOther').replace(
          '{count}',
          String(toolCount)
        )
      : null;
  if (workedMs !== undefined && tools) {
    return t('chat.tools.workedFor')
      .replace('{duration}', formatElapsed(workedMs))
      .replace('{calls}', tools);
  }
  if (reasoningCount > 0 && tools)
    return t('chat.tools.reasoningWithCalls').replace('{calls}', tools);
  if (reasoningCount > 0) return t('chat.tools.reasoning');
  return tools ?? t('chat.tools.activity');
}

/**
 * One disclosure for everything the agent did between the user's input and its
 * answer: reasoning and tool calls together, in the order they happened.
 *
 * It replaces a chain-of-thought wrapper that split the same run into separate
 * reasoning and tool groups. A turn that alternates — think, call, think, call —
 * then rendered as a stack of unrelated collapsibles, each with its own trigger,
 * and the answer drowned among them. As one group the message reads input →
 * work → answer however the work interleaved.
 *
 * Open while the work is live (`running`, the running turn's tail, or
 * `requires-action` for a tool parked on an approval, whose decision card lives
 * inside), closed once it settles so the answer leads. The first manual toggle wins from then on.
 */
export const ActivityGroup: FC<PropsWithChildren<{ group: ActivityGroupPart }>> = ({
  group,
  children,
}) => {
  const { indices } = group;
  // Numbers, not arrays, so the selectors are stable across renders.
  const toolCount = useAuiState(
    s => indices.filter(i => s.message.parts[i]?.type === 'tool-call').length
  );
  const reasoningCount = useAuiState(
    s => indices.filter(i => s.message.parts[i]?.type === 'reasoning').length
  );
  // Between steps — a tool has returned, the next inference has not started —
  // every part in the group is complete while the turn is not. Without this the
  // group would close and reopen on every round trip. It stays open until the
  // turn moves past it (the answer starts streaming) or ends.
  const isTail = useAuiState(
    s => s.message.status?.type === 'running' && indices.at(-1) === s.message.parts.length - 1
  );
  // `GroupedParts` takes its status from the final part. A completed call after
  // a parked approval would otherwise collapse the approval card out of sight.
  const requiresAction = useAuiState(s =>
    indices.some(i => s.message.parts[i]?.status?.type === 'requires-action')
  );
  // The turn's wall time (`chat_done.timing.total_ms`), credited to this group
  // only when it holds every tool call in the message: a message with several
  // activity groups cannot say which share of the time each one took.
  const messageToolCount = useAuiState(
    s => s.message.parts.filter(part => part?.type === 'tool-call').length
  );
  const totalStreamTime = useAuiState(
    s =>
      (s.message.metadata as { timing?: { totalStreamTime?: number } } | undefined)?.timing
        ?.totalStreamTime
  );
  const { t } = useT();
  const [userOpen, setUserOpen] = useState<boolean | null>(null);

  const running = group.status.type === 'running' || isTail;
  const live = running || group.status.type === 'requires-action' || requiresAction;

  // Preserve the dedicated reasoning trace for runs that contain no tools. The
  // combined disclosure is needed only when tools and reasoning interleave.
  if (toolCount === 0 && reasoningCount > 0) {
    return <OpenHumanReasoningGroup indices={indices} running={running} />;
  }

  return (
    <ToolGroupRoot variant="ghost" open={userOpen ?? live} onOpenChange={setUserOpen}>
      <ToolGroupTrigger
        count={toolCount}
        label={activityGroupLabel(
          reasoningCount,
          toolCount,
          t,
          !live && toolCount === messageToolCount ? totalStreamTime : undefined
        )}
        active={running}
      />
      <ToolGroupContent>{children}</ToolGroupContent>
    </ToolGroupRoot>
  );
};
