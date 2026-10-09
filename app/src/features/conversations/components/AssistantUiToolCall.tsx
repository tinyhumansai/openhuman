import type { ToolCallMessagePart, ToolCallMessagePartProps } from '@assistant-ui/react';
import type { FC, ReactNode } from 'react';

import {
  ToolCall,
  type ToolCallOutcome,
} from '../../../components/assistant-ui/elements/tool-call';
import { useDisclosure } from '../../../components/assistant-ui/lib/useDisclosure';
import { useT } from '../../../lib/i18n/I18nContext';
import { readOpenHumanToolArtifact } from '../../../providers/assistantUiMessages';
import type {
  ToolFailureExplanation,
  ToolTimelineEntryStatus,
} from '../../../store/chatRuntimeSlice';
import { openUrl } from '../../../utils/openUrl';
import { ToolFailureCard } from '../aui/ToolFailureCard';
import { useLiveElapsed, useRunningSince } from '../aui/useLiveElapsed';
import { McpUiBody } from '../tools/mcpUi/McpUiBody';
import { readMcpUiPresentation } from '../tools/mcpUi/types';
import { isSearchBalanceError } from '../tools/parseWebSearchResult';
import {
  FetchBody,
  FileBody,
  SearchBalanceHint,
  ShellBody,
  WebSearchBody,
} from '../tools/ToolBodies';
import { hasDisplayValue, parsedValue, ToolDataView } from '../tools/ToolDataView';
import { ToolIcon } from '../tools/ToolIcon';
import { describeToolCall, parseToolArgs, toolLabel } from '../tools/toolPresentation';

/** `1234` → "1.2s", `850` → "850ms", `75000` → "1m 15s". */
export function formatElapsed(ms: number): string {
  if (ms < 1000) return `${Math.max(0, Math.round(ms))}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  const minutes = Math.floor(ms / 60_000);
  const seconds = Math.round((ms % 60_000) / 1000);
  return `${minutes}m ${seconds}s`;
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
  /** Server label; used only for tools the presentation registry cannot describe. */
  displayName?: string;
  detail?: string;
  elapsedMs?: number;
  /** Machine-readable result from the core (e.g. structured web-search hits). */
  structured?: unknown;
  failure?: ToolFailureExplanation;
  /**
   * The call is parked on the user — an ApprovalGate request, or a sub-agent
   * that asked a question. Renders as in-flight (it *is* unfinished) but says
   * what it is actually waiting for.
   */
  awaitingUser?: boolean;
  /** Decision row / connect affordance, rendered under the call's header. */
  footer?: ReactNode;
  /**
   * Stable identity (the tool-call id) under which the user's open/closed
   * choice is remembered across remounts. See `useDisclosure`.
   */
  disclosureKey?: string;
}

/**
 * One tool call, rendered with assistant-ui's tool-call element.
 *
 * The icon, the label (in both tenses, which the element swaps between as
 * the call settles) and the target chip all come from the presentation
 * registry, so every surface names a call the same way. The panel expands
 * into the tool's own assistant-ui element (search results, terminal, diff,
 * page preview) or the generic Request / Result view.
 */
export function AssistantUiToolCallCard({
  toolName,
  args,
  argsText,
  result,
  status,
  displayName,
  detail,
  elapsedMs,
  structured,
  failure,
  awaitingUser = false,
  footer,
  disclosureKey,
}: AssistantUiToolCallCardProps) {
  const { t } = useT();
  // Defaults open while the call is waiting on the user (an approval or a
  // sub-agent question needs to be visible without an extra click); once the
  // user has explicitly toggled it, that choice is remembered by
  // `disclosureKey` (the tool-call id) so it survives assistant-ui's remounts
  // — a virtualized history, a thread switch, a part whose shape changes.
  const [open, setOpen] = useDisclosure(
    disclosureKey ? `tool:${disclosureKey}` : undefined,
    awaitingUser
  );
  const running =
    awaitingUser ||
    (status ? status === 'running' || status === 'awaiting_user' : result === undefined);
  const effectiveStatus: ToolTimelineEntryStatus =
    status ?? (awaitingUser ? 'awaiting_user' : running ? 'running' : 'success');
  const input = hasDisplayValue(args) ? args : parsedValue(argsText ?? '');
  const parsedArgs = parseToolArgs(input);
  const output = result === '' && status && !running ? t('conversations.tools.noOutput') : result;
  const presentation = describeToolCall({
    name: toolName,
    args: parsedArgs,
    status: effectiveStatus,
    serverLabel: displayName,
    serverDetail: detail,
  });
  const activeLabel = toolLabel({ ...presentation, tense: 'active' }, t);
  const doneLabel = toolLabel({ ...presentation, tense: 'done' }, t);
  const failed = status === 'error';
  const awaiting = awaitingUser || status === 'awaiting_user';
  // Live clock while the tool runs (not while it waits on the user, whose
  // time is not the tool's). Tool parts carry no start timestamp, so the clock
  // starts when the row first renders running — after a remount it restarts.
  const liveRunning = running && !awaiting;
  const liveMs = useLiveElapsed(useRunningSince(liveRunning), liveRunning);
  const outcome: ToolCallOutcome = failed
    ? 'error'
    : status === 'cancelled'
      ? 'cancelled'
      : awaiting
        ? 'awaiting'
        : 'success';
  const statusText =
    outcome === 'error'
      ? t('conversations.tools.status.failed')
      : outcome === 'cancelled'
        ? t('conversations.tools.status.cancelled')
        : outcome === 'awaiting'
          ? t('conversations.tools.status.awaiting')
          : running
            ? t('conversations.tools.status.running')
            : t('conversations.tools.status.done');
  // Running and done are carried by the spinner / check; they stay readable
  // to a screen reader. The states that need attention are spelled out.
  const statusVisible = outcome !== 'success';

  const searchBody =
    presentation.body === 'webSearch'
      ? WebSearchBody({ args: parsedArgs, result: output, structured, searching: running, t })
      : null;
  const richBody = running
    ? null
    : presentation.body === 'shell'
      ? ShellBody({ args: parsedArgs, result: output, failed, t })
      : presentation.body === 'webFetch'
        ? FetchBody({ args: parsedArgs, result: output, t, onOpenExternal: openExternal })
        : presentation.body === 'file'
          ? FileBody({ args: parsedArgs, result: output })
          : null;
  const showOutput = !searchBody && hasDisplayValue(parsedValue(output));
  const mcpUi = !running && readMcpUiPresentation(structured) ? structured : undefined;
  // Managed search out of balance: say so plainly instead of leaving the user
  // to decode the raw error.
  const balanceHint =
    failed &&
    presentation.category === 'web' &&
    (isSearchBalanceError(output) || isSearchBalanceError(failure));

  return (
    <ToolCall
      data-testid="assistant-ui-tool-call"
      data-tool-name={toolName}
      className="max-w-none"
      label={doneLabel}
      activeLabel={activeLabel}
      // The web-search element shows the query as its own pill.
      query={searchBody ? undefined : presentation.chip}
      running={running}
      outcome={outcome}
      open={open}
      onOpenChange={setOpen}
      icon={<ToolIcon presentation={presentation} className="text-foreground/45 size-3.5" />}
      requestLabel={t('conversations.subagent.input')}
      resultLabel={t('conversations.subagent.output')}
      request={
        !richBody && hasDisplayValue(input) ? (
          <div data-testid="assistant-ui-tool-input">
            <ToolDataView value={input} />
          </div>
        ) : undefined
      }
      result={
        !richBody && showOutput ? (
          <div data-testid="assistant-ui-tool-output">
            <ToolDataView value={output} />
          </div>
        ) : undefined
      }
      meta={
        <>
          <span
            data-testid="tool-call-status"
            className={
              !statusVisible || (running && outcome !== 'awaiting')
                ? 'sr-only'
                : outcome === 'awaiting'
                  ? 'text-amber-600 dark:text-amber-400'
                  : undefined
            }>
            {statusText}
          </span>
          {elapsedMs != null && !running ? (
            <span data-testid="tool-call-elapsed" className="tabular-nums">
              {formatElapsed(elapsedMs)}
            </span>
          ) : liveMs !== undefined && liveMs >= 1000 ? (
            <span data-testid="tool-call-elapsed-live" className="tabular-nums">
              {formatElapsed(Math.floor(liveMs / 1000) * 1000)}
            </span>
          ) : null}
        </>
      }
      aside={
        <>
          {failed && failure ? (
            <div className="ps-5.5 pt-1 pb-2">
              <ToolFailureCard
                toolName={toolName}
                target={detail ?? displayName}
                failure={failure}
              />
            </div>
          ) : null}
          {balanceHint ? (
            <div className="ps-5.5 pt-1 pb-2">
              <SearchBalanceHint t={t} />
            </div>
          ) : null}
          {footer ? <div className="ps-5.5">{footer}</div> : null}
          {/* Search results are the call's whole point: visible without
              opening the disclosure, as in assistant-ui's own web-search. */}
          {searchBody ? <div className="ps-5.5 pt-1 pb-2">{searchBody}</div> : null}
          {mcpUi !== undefined ? (
            <div className="ps-5.5 pt-1 pb-2">
              <McpUiBody structured={mcpUi} />
            </div>
          ) : null}
        </>
      }>
      {richBody ? <div className="mt-2 ps-5.5">{richBody}</div> : undefined}
    </ToolCall>
  );
}

function openExternal(url: string): void {
  void openUrl(url).catch(() => undefined);
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
 * The part's `artifact` carries what the core said about the call (its label
 * for a dynamic tool, the duration, a structured result). The adapter used to
 * drop all of it, so the card guessed a label from the tool name.
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
  const artifact = readOpenHumanToolArtifact(props.artifact);
  return (
    <AssistantUiToolCallCard
      toolName={props.toolName}
      args={props.args}
      argsText={props.argsText}
      result={envelope ? envelope.value : props.result}
      status={envelope?.status}
      failure={envelope?.failure}
      displayName={artifact?.displayName}
      detail={artifact?.detail}
      elapsedMs={artifact?.elapsedMs}
      structured={artifact?.structured}
      awaitingUser={isApprovalPending(props.approval)}
      footer={props.approvalCard}
      disclosureKey={props.toolCallId}
    />
  );
};
