import type { ToolCallMessagePart, ToolCallMessagePartProps } from '@assistant-ui/react';
import { CheckIcon, ChevronDownIcon, CircleXIcon, Loader2Icon, WrenchIcon } from 'lucide-react';
import type { FC, ReactNode } from 'react';

import { cn } from '../../../components/assistant-ui/lib/utils';
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '../../../components/assistant-ui/ui/collapsible';
import type {
  ToolFailureExplanation,
  ToolTimelineEntryStatus,
} from '../../../store/chatRuntimeSlice';
import { formatToolName } from '../../../utils/toolTimelineFormatting';
import { BubbleMarkdown } from './AgentMessageBubble';
import { ToolFailureLines } from './ToolFailureLines';

function friendlyLabel(key: string): string {
  return key
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/[_-]+/g, ' ')
    .replace(/^./, char => char.toUpperCase());
}

function parsedValue(value: unknown): unknown {
  if (typeof value !== 'string') return value;
  const trimmed = value.trim();
  if (!(trimmed.startsWith('{') || trimmed.startsWith('['))) return value;
  try {
    return JSON.parse(trimmed);
  } catch {
    return value;
  }
}

function hasDisplayValue(value: unknown): boolean {
  if (value === undefined || value === null || value === '') return false;
  if (Array.isArray(value)) return value.length > 0;
  if (typeof value === 'object') return Object.keys(value as object).length > 0;
  return true;
}

function ToolDataView({ value }: { value: unknown }) {
  const parsed = parsedValue(value);
  if (Array.isArray(parsed)) {
    return (
      <ul className="space-y-1 text-xs">
        {parsed.map((item, index) => (
          <li key={index} className="bg-muted/50 rounded-md px-2 py-1.5">
            <ToolDataView value={item} />
          </li>
        ))}
      </ul>
    );
  }
  if (parsed && typeof parsed === 'object') {
    const entries = Object.entries(parsed);
    for (const key of ['content', 'output', 'result', 'message', 'query', 'q']) {
      const semantic = entries.find(([candidate]) => candidate === key)?.[1];
      if (hasDisplayValue(semantic)) return <ToolDataView value={semantic} />;
    }
    return (
      <dl className="divide-border bg-muted/40 divide-y rounded-md px-2 text-xs">
        {entries.map(([key, item]) => (
          <div key={key} className="grid grid-cols-[minmax(7rem,auto)_1fr] gap-3 py-1.5">
            <dt className="text-muted-foreground font-medium">{friendlyLabel(key)}</dt>
            <dd className="min-w-0 wrap-break-word">
              <ToolDataView value={item} />
            </dd>
          </div>
        ))}
      </dl>
    );
  }
  if (typeof parsed === 'boolean') return <span>{parsed ? 'Yes' : 'No'}</span>;
  if (typeof parsed === 'string') return <BubbleMarkdown content={parsed} />;
  return <span className="whitespace-pre-wrap">{String(parsed ?? '')}</span>;
}

function inferredToolLabel(toolName: string, running: boolean, args: unknown, result: unknown) {
  const lowerName = toolName.toLowerCase();
  const parsedArgs = parsedValue(args);
  const argKeys =
    parsedArgs && typeof parsedArgs === 'object' && !Array.isArray(parsedArgs)
      ? Object.keys(parsedArgs as object).map(key => key.toLowerCase())
      : [];
  const renderedResult = typeof result === 'string' ? result : JSON.stringify(result ?? '');
  const looksLikeSearch =
    lowerName.includes('search') ||
    argKeys.some(key => ['query', 'q', 'search_query'].includes(key)) ||
    /(?:^|\n)#?\s*search results\b/i.test(renderedResult);
  const looksLikeFetch =
    lowerName.includes('fetch') ||
    argKeys.some(key => ['url', 'uri'].includes(key)) ||
    /\bstatus=\d{3}\s+url=/i.test(renderedResult);
  if (looksLikeSearch) return running ? 'Searching the web' : 'Searched the web';
  if (looksLikeFetch) return running ? 'Fetching from the web' : 'Fetched from the web';
  return formatToolName(toolName);
}

/**
 * Is this approval still the user's to answer?
 *
 * A resolved one keeps its `approval` object (with `approved` / `resolution`
 * filled in) so the transcript can show what was decided — offering the buttons
 * again would let a second decision race the first.
 */
export function isApprovalPending(approval: ToolCallMessagePart['approval']): boolean {
  return approval != null && approval.approved === undefined && approval.resolution === undefined;
}

export interface AssistantUiToolCallCardProps {
  toolName: string;
  args?: unknown;
  argsText?: string;
  result?: unknown;
  status?: ToolTimelineEntryStatus;
  displayName?: string;
  detail?: string;
  elapsedMs?: number;
  failure?: ToolFailureExplanation;
  /**
   * The call is parked on the user — an ApprovalGate request, or a sub-agent
   * that asked a question. Renders as in-flight (it *is* unfinished) but says
   * what it is actually waiting for.
   */
  awaitingUser?: boolean;
  /** Decision row / connect affordance, rendered under the call's header. */
  footer?: ReactNode;
}

/** The single assistant-ui tool-call presentation used at every nesting level. */
export function AssistantUiToolCallCard({
  toolName,
  args,
  argsText,
  result,
  status,
  displayName,
  detail,
  elapsedMs,
  failure,
  awaitingUser = false,
  footer,
}: AssistantUiToolCallCardProps) {
  const running =
    awaitingUser ||
    (status ? status === 'running' || status === 'awaiting_user' : result === undefined);
  const input = hasDisplayValue(args) ? args : parsedValue(argsText ?? '');
  const output = result === '' && status && !running ? 'No output' : parsedValue(result);
  const suppliedLabel = displayName?.trim();
  const label =
    suppliedLabel && suppliedLabel.toLowerCase() !== 'tool'
      ? suppliedLabel
      : inferredToolLabel(toolName, running, args, result);
  // `awaiting input` was previously reachable only via `status`, which the
  // adapter forwards for `error` / `cancelled` alone — so the label could never
  // render for the case it was written for. A parked call now says so.
  const statusLabel =
    status === 'error'
      ? 'failed'
      : status === 'cancelled'
        ? 'cancelled'
        : awaitingUser || status === 'awaiting_user'
          ? 'awaiting input'
          : running
            ? 'running'
            : 'done';
  const failed = status === 'error';
  // `failed` gates the failure-explanation block, which only an `error` carries.
  // The icon is a wider question: a cancelled call did not succeed either, and
  // before the adapter forwarded a status this branch was unreachable, so the
  // check icon sat next to the word "cancelled".
  const terminalNonSuccess = failed || status === 'cancelled';

  return (
    <Collapsible
      data-slot="aui_openhuman-tool-call"
      data-testid="assistant-ui-tool-call"
      defaultOpen={running}
      data-awaiting-user={awaitingUser ? 'true' : undefined}
      className={cn(
        'border-border/60 dark:border-muted-foreground/15 rounded-xl border',
        running && 'border-dashed'
      )}>
      <CollapsibleTrigger className="group/tool text-muted-foreground hover:text-foreground flex w-full items-center gap-2 px-3 py-2 text-sm transition-colors">
        <WrenchIcon className="size-4 shrink-0" />
        <span className="text-foreground text-start font-medium">{label}</span>
        {detail ? (
          <span className="bg-muted min-w-0 truncate rounded px-1.5 py-0.5 font-mono text-[11px]">
            {detail}
          </span>
        ) : null}
        <span className="flex shrink-0 items-center gap-1 text-[11px]">
          {running ? (
            <Loader2Icon className="size-3 animate-spin [animation-duration:0.6s]" />
          ) : terminalNonSuccess ? (
            <CircleXIcon className="size-3.5" />
          ) : (
            <CheckIcon className="size-3.5" />
          )}
          {statusLabel}
          {elapsedMs != null && !running ? (
            <span className="tabular-nums">
              {elapsedMs >= 1000 ? `${(elapsedMs / 1000).toFixed(1)}s` : `${elapsedMs}ms`}
            </span>
          ) : null}
        </span>
        <ChevronDownIcon className="ml-auto size-4 shrink-0 -rotate-90 transition-transform group-data-[state=open]/tool:rotate-0" />
      </CollapsibleTrigger>
      {failed && failure ? (
        <div className="px-3 pb-2">
          <ToolFailureLines failure={failure} />
        </div>
      ) : null}
      {/* Outside `CollapsibleContent` on purpose: a decision the turn is
          blocked on must not be hidden behind a disclosure the user has to
          find and open. */}
      {footer}
      <CollapsibleContent className="space-y-2 px-3 pb-3">
        {hasDisplayValue(input) ? (
          <div data-testid="assistant-ui-tool-input">
            <p className="text-muted-foreground mb-1 text-[11px] font-medium uppercase">Input</p>
            <div className="max-h-48 overflow-auto">
              <ToolDataView value={input} />
            </div>
          </div>
        ) : null}
        {hasDisplayValue(output) ? (
          <div data-testid="assistant-ui-tool-output">
            <p className="text-muted-foreground mb-1 text-[11px] font-medium uppercase">Output</p>
            <div className="max-h-64 overflow-auto">
              <ToolDataView value={output} />
            </div>
          </div>
        ) : null}
      </CollapsibleContent>
    </Collapsible>
  );
}

/**
 * Terminal status carried inside a settled tool part's `result`.
 *
 * assistant-ui's tool-call part has no status field, so `toolPart` puts the
 * status there for a tool that failed or was cancelled (`value` holds the real
 * output when there was one). Without unwrapping it here the card fell back to
 * `result !== undefined`, which reads as success — a failed tool rendered
 * "done" with a check.
 */
function toolStatusEnvelope(
  result: unknown
):
  | { status: ToolTimelineEntryStatus; failure?: ToolFailureExplanation; value?: unknown }
  | undefined {
  if (!result || typeof result !== 'object' || Array.isArray(result)) return undefined;
  const candidate = result as { status?: unknown; failure?: unknown; value?: unknown };
  return candidate.status === 'error' || candidate.status === 'cancelled'
    ? {
        status: candidate.status as ToolTimelineEntryStatus,
        failure: candidate.failure as ToolFailureExplanation | undefined,
        ...('value' in candidate ? { value: candidate.value } : {}),
      }
    : undefined;
}

/**
 * One tool call in the assistant-ui transcript.
 *
 * `approval` is supplied by assistant-ui on every tool part; this component used
 * to destructure four fields and drop the rest, which is why a parked call
 * rendered as an ordinary running one with no way to answer it.
 *
 * The decision surface itself is passed in rather than built here. It is
 * `ApprovalRequestCard`, which needs the thread id and the store's
 * `PendingApproval` — neither of which belongs in this file, and both of which
 * `ChatToolParts` already resolves for the `composio_connect` route.
 */
export const OpenHumanToolCall: FC<
  ToolCallMessagePartProps & {
    /** Decision surface for a parked call; rendered under the call's header. */
    approvalCard?: ReactNode;
  }
> = props => {
  const envelope = toolStatusEnvelope(props.result);
  return (
    <AssistantUiToolCallCard
      toolName={props.toolName}
      args={props.args}
      argsText={props.argsText}
      result={envelope ? envelope.value : props.result}
      status={envelope?.status}
      failure={envelope?.failure}
      awaitingUser={isApprovalPending(props.approval)}
      footer={props.approvalCard}
    />
  );
};
