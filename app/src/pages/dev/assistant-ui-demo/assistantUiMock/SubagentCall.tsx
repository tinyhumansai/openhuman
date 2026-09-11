/**
 * Renders a `task` tool call — a delegation to a subagent — as its own block
 * rather than through the generic `ToolFallback`.
 *
 * A delegation is not shaped like an ordinary call: it has an agent identity, a
 * running list of nested steps it took, and a written report at the end. Folding
 * that into the fallback's args/result JSON pair loses the thing worth seeing.
 *
 * And what is worth seeing is that the parent turn is *not* waiting. Delegation
 * is asynchronous: the turn dispatches and carries on, so these steps arrive
 * underneath prose that was written after them. The running state says so
 * explicitly — a "running" pill and a ticking clock — because a spinner alone
 * reads as "the turn is blocked here", which is the opposite of what happens.
 *
 * Styling stays on the shadcn semantic tokens the rest of the vendored set
 * uses, so this follows the app theme in both modes.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import type { ThreadGroupPart } from '@/components/assistant-ui/thread';
import { ToolFallback } from '@/components/assistant-ui/tool-fallback';
import {
  ToolGroupContent,
  ToolGroupRoot,
  ToolGroupTrigger,
} from '@/components/assistant-ui/tool-group';
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/assistant-ui/ui/collapsible';
import type { ToolCallMessagePartComponent } from '@assistant-ui/react';
import { CheckIcon, ChevronDownIcon, Loader2Icon, WorkflowIcon } from 'lucide-react';
import type { FC, PropsWithChildren } from 'react';

import type { MockSubagentResult } from './mockScript';

/** Narrow an untyped payload to the shape this demo's script produces. */
function asSubagentResult(value: unknown): MockSubagentResult | undefined {
  if (typeof value !== 'object' || value === null) return undefined;
  const candidate = value as Partial<MockSubagentResult>;
  if (typeof candidate.subagent !== 'string' || !Array.isArray(candidate.steps)) return undefined;
  return candidate as MockSubagentResult;
}

/**
 * A delegation reports from two places over its life. While it runs there is no
 * result — that is what keeps the runtime's status `running` — and progress
 * arrives on the streaming args under `progress`. When it finishes, `result`
 * carries the same shape plus the report. Reading result first means a finished
 * call renders from the authoritative payload.
 */
function readState(
  args: unknown,
  result: unknown
): { state: MockSubagentResult | undefined; running: boolean } {
  const done = asSubagentResult(result);
  if (done) return { state: done, running: done.status !== 'complete' };
  const progress =
    typeof args === 'object' && args !== null
      ? asSubagentResult((args as { progress?: unknown }).progress)
      : undefined;
  return { state: progress, running: true };
}

export const SubagentCall: ToolCallMessagePartComponent = ({ args, result }) => {
  const { state: parsed, running } = readState(args, result);
  const elapsed = parsed?.elapsedSeconds;
  const description = (args as { description?: string } | undefined)?.description;
  const name =
    parsed?.subagent ??
    (args as { subagent_type?: string } | undefined)?.subagent_type ??
    'subagent';

  return (
    <Collapsible
      data-slot="aui_subagent-call"
      defaultOpen
      className={cn(
        'aui-subagent-call border-border/60 dark:border-muted-foreground/15 rounded-xl border',
        running && 'border-dashed'
      )}>
      <CollapsibleTrigger className="group/subagent text-muted-foreground hover:text-foreground flex w-full items-center gap-2 px-3 py-2 text-sm transition-colors">
        <WorkflowIcon className="size-4 shrink-0" />
        <span className="text-start leading-none">
          Delegated to <b className="text-foreground">{name}</b>
        </span>
        {running ? (
          <span className="bg-muted text-muted-foreground flex shrink-0 items-center gap-1.5 rounded-full px-2 py-0.5 text-[11px] leading-none">
            <Loader2Icon className="size-3 animate-spin [animation-duration:0.6s]" />
            running
            {elapsed !== undefined && <span className="tabular-nums">{elapsed.toFixed(1)}s</span>}
          </span>
        ) : (
          <span className="text-muted-foreground flex shrink-0 items-center gap-1.5 text-[11px] leading-none">
            <CheckIcon className="size-3.5" />
            {elapsed !== undefined && <span className="tabular-nums">{elapsed.toFixed(1)}s</span>}
          </span>
        )}
        <ChevronDownIcon className="ml-auto size-4 shrink-0 -rotate-90 transition-transform group-data-[state=open]/subagent:rotate-0" />
      </CollapsibleTrigger>

      <CollapsibleContent className="px-3 pb-3">
        {description && <p className="text-muted-foreground mb-2 text-xs">{description}</p>}

        <ol className="flex flex-col gap-1.5">
          {parsed?.steps.map((step, i) => (
            <li
              key={`${step.tool}-${i}`}
              className="text-muted-foreground flex items-baseline gap-2 text-xs">
              <span className="bg-muted text-foreground rounded px-1.5 py-0.5 font-mono text-[11px]">
                {step.tool}
              </span>
              <span className="min-w-0 break-words">{step.detail}</span>
            </li>
          ))}
        </ol>

        {running && (
          <p className="text-muted-foreground mt-2 text-xs italic">
            Still running — the turn did not wait for it.
          </p>
        )}

        {parsed?.report && (
          <p
            className={cn(
              'text-foreground border-border/60 dark:border-muted-foreground/15 mt-3 border-t pt-3 text-sm leading-relaxed'
            )}>
            {parsed.report}
          </p>
        )}
      </CollapsibleContent>
    </Collapsible>
  );
};

export default SubagentCall;

/**
 * Drop-in for `Thread`'s `components.ToolFallback` seam: routes a `task` call
 * to {@link SubagentCall} and leaves every other tool to the stock fallback.
 *
 * Using the seam rather than editing `thread.tsx` keeps the vendored component
 * set unmodified, so it can still be re-pulled from the registry.
 */
export const MockToolFallback: ToolCallMessagePartComponent = props =>
  props.toolName === 'task' ? <SubagentCall {...props} /> : <ToolFallback {...props} />;

/**
 * Drop-in for `Thread`'s `components.ToolGroup` seam.
 *
 * Identical to the stock group except that a group holding work still in flight
 * opens itself. Collapsed-by-default is right for a finished trace, but it
 * hides the one thing a dispatched delegation needs to show: that it is still
 * running while the answer below it streams. `defaultOpen` only applies on
 * mount, so the group opens once and the reader can still collapse it.
 */
export const MockToolGroup: FC<PropsWithChildren<{ group: ThreadGroupPart }>> = ({
  group,
  children,
}) => {
  const running = group.status.type === 'running';
  return (
    <ToolGroupRoot variant="ghost" defaultOpen={running}>
      <ToolGroupTrigger count={group.indices.length} active={running} />
      <ToolGroupContent>{children}</ToolGroupContent>
    </ToolGroupRoot>
  );
};
