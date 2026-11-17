'use client';

/**
 * assistant-ui's tool-fallback element: the default renderer for any tool
 * call the toolkit does not register its own element for, plus the shared
 * approval bar every gated call renders while it waits on the user.
 *
 * Vendored from the assistant-ui `tool-fallback` registry item
 * (https://r.assistant-ui.com/styles/base-nova/tool-fallback.json —
 * `elements/tool-fallback.aui.tsx` upstream). Changes from upstream:
 * - `cn` import path and Radix collapsible from `../ui/collapsible`.
 * - `formatUnknownValue` handles `Error` values and an unserializable
 *   fallback the same way upstream does, but the cancelled-call result stays
 *   hidden (`!isCancelled && <ToolFallbackResult .../>`) rather than always
 *   rendered — a cancelled call's stale result would otherwise read as a
 *   real one.
 * - Upstream's free-text answer path (`Textarea`, `toolApprovalAcceptsText`,
 *   the `isQuestion`/`promptText` branches) and the voice-session lock
 *   (`useAuiState(s => s.thread.voice)`) are now ported — the
 *   `@assistant-ui/react` / `@assistant-ui/core` pin WS-A landed
 *   (`^0.15.21` / `^0.3.20`) exports `toolApprovalAcceptsText` and carries
 *   `approval.display` / `approval.prompt` / `approval.allowFreeform` and a
 *   `text` member on `ToolApprovalResponse`.
 *   **Still not ported:** upstream's `approval.dismissible` / "Dismiss"
 *   affordance — that field is not on this pin's `approval` type (only
 *   `id`/`prompt`/`display`/`allowFreeform`/the decision fields are), so a
 *   further version bump is needed before a question-mode request can offer
 *   one; `dismissButton` stays `null` until then.
 *
 *   `ChatToolFallback` (`features/conversations/components/ChatToolParts.tsx`)
 *   intercepts OpenHuman's own gated-approval path before it ever reaches
 *   this element — `GatedToolCall` renders `ApprovalCardAdapter` for a
 *   `status.type === 'requires-action'` call with `part.approval` set, and
 *   `ComposioConnectCall` handles the connector-auth case — so this
 *   `ToolFallbackApproval` bar is reached only by an interrupt/approval this
 *   app does not already have its own card for (a `human()`/HITL pause or a
 *   raw MCP elicitation the toolkit has no dedicated entry for). WS-B's own
 *   adapters (`ApprovalCardAdapter.tsx`, `PermissionGrantAdapter.tsx`,
 *   `ElicitationAdapter.tsx`) cover the OpenHuman-specific gate/permission/
 *   elicitation surfaces directly; this element stays the generic upstream
 *   fallback underneath them, kept in sync with upstream rather than
 *   duplicating that logic.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import { Button } from '@/components/assistant-ui/ui/button';
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/assistant-ui/ui/collapsible';
import { Textarea } from '@/components/assistant-ui/ui/textarea';
import { useT } from '@/lib/i18n/I18nContext';
import {
  toolApprovalAcceptsText,
  type ToolApprovalOption,
  type ToolCallMessagePart,
  type ToolCallMessagePartComponent,
  type ToolCallMessagePartProps,
  type ToolCallMessagePartStatus,
  useAuiState,
  useScrollLock,
  useToolCallElapsed,
} from '@assistant-ui/react';
import { AlertCircleIcon, CheckIcon, ChevronDownIcon, LoaderIcon, XCircleIcon } from 'lucide-react';
import { memo, useCallback, useRef, useState } from 'react';

const ANIMATION_DURATION = 200;

const pressable = 'active:scale-[0.98]';

export type ToolFallbackRootProps = Omit<
  React.ComponentProps<typeof Collapsible>,
  'open' | 'onOpenChange'
> & { open?: boolean; onOpenChange?: (open: boolean) => void; defaultOpen?: boolean };

function ToolFallbackRoot({
  className,
  open: controlledOpen,
  onOpenChange: controlledOnOpenChange,
  defaultOpen = false,
  children,
  ...props
}: ToolFallbackRootProps) {
  const collapsibleRef = useRef<HTMLDivElement>(null);
  const [uncontrolledOpen, setUncontrolledOpen] = useState(defaultOpen);
  const lockScroll = useScrollLock(collapsibleRef, ANIMATION_DURATION);

  const isControlled = controlledOpen !== undefined;
  const isOpen = isControlled ? controlledOpen : uncontrolledOpen;

  const handleOpenChange = useCallback(
    (open: boolean) => {
      lockScroll();
      if (!isControlled) {
        setUncontrolledOpen(open);
      }
      controlledOnOpenChange?.(open);
    },
    [lockScroll, isControlled, controlledOnOpenChange]
  );

  return (
    <Collapsible
      ref={collapsibleRef}
      data-slot="tool-fallback-root"
      open={isOpen}
      onOpenChange={handleOpenChange}
      className={cn('aui-tool-fallback-root group/tool-fallback-root w-full', className)}
      style={{ '--animation-duration': `${ANIMATION_DURATION}ms` } as React.CSSProperties}
      {...props}>
      {children}
    </Collapsible>
  );
}

type ToolStatus = ToolCallMessagePartStatus['type'];

const statusIconMap: Record<ToolStatus, React.ElementType> = {
  running: LoaderIcon,
  complete: CheckIcon,
  incomplete: XCircleIcon,
  'requires-action': AlertCircleIcon,
};

const formatToolDuration = (ms: number) => {
  if (ms < 1000) return '<1s';
  const seconds = ms / 1000;
  if (seconds < 10) return `${(Math.floor(seconds * 10) / 10).toFixed(1)}s`;
  if (seconds < 60) return `${Math.floor(seconds)}s`;
  return `${Math.floor(seconds / 60)}m ${Math.floor(seconds % 60)}s`;
};

function ToolFallbackDuration({ className, ...props }: React.ComponentProps<'span'>) {
  const elapsedMs = useToolCallElapsed();
  if (elapsedMs === undefined) return null;

  return (
    <span
      data-slot="tool-fallback-duration"
      className={cn(
        'aui-tool-fallback-duration text-muted-foreground text-xs tabular-nums',
        className
      )}
      {...props}>
      {formatToolDuration(elapsedMs)}
    </span>
  );
}

function ToolFallbackTrigger({
  toolName,
  status,
  className,
  ...props
}: React.ComponentProps<typeof CollapsibleTrigger> & {
  toolName: string;
  status?: ToolCallMessagePartStatus;
}) {
  const statusType = status?.type ?? 'complete';
  const isRunning = statusType === 'running';
  const isCancelled = status?.type === 'incomplete' && status.reason === 'cancelled';

  const Icon = statusIconMap[statusType];
  const label = isCancelled ? 'Cancelled tool' : 'Used tool';

  return (
    <CollapsibleTrigger
      data-slot="tool-fallback-trigger"
      className={cn(
        'aui-tool-fallback-trigger group/trigger text-muted-foreground hover:text-foreground flex w-fit max-w-full origin-left items-center gap-2 py-1.5 text-sm transition-[color,scale] active:scale-[0.98]',
        className
      )}
      {...props}>
      <Icon
        data-slot="tool-fallback-trigger-icon"
        className={cn(
          'aui-tool-fallback-trigger-icon size-4 shrink-0',
          isCancelled && 'text-muted-foreground',
          isRunning && 'animate-spin [animation-duration:0.6s]'
        )}
      />
      <span
        data-slot="tool-fallback-trigger-label"
        className={cn(
          'aui-tool-fallback-trigger-label-wrapper relative inline-block text-start leading-none',
          isCancelled && 'text-muted-foreground line-through'
        )}>
        <span>
          {label}: <b>{toolName}</b>
        </span>
        {isRunning && (
          <span
            aria-hidden
            data-slot="tool-fallback-trigger-shimmer"
            className="aui-tool-fallback-trigger-shimmer shimmer pointer-events-none absolute inset-0 motion-reduce:animate-none">
            {label}: <b>{toolName}</b>
          </span>
        )}
      </span>
      <ToolFallbackDuration />
      <ChevronDownIcon
        data-slot="tool-fallback-trigger-chevron"
        className={cn(
          'aui-tool-fallback-trigger-chevron size-4 shrink-0',
          'transition-transform duration-(--animation-duration) ease-[cubic-bezier(0.32,0.72,0,1)] motion-reduce:transition-none',
          '-rotate-90',
          'group-data-open/trigger:rotate-0',
          'group-data-panel-open/trigger:rotate-0'
        )}
      />
    </CollapsibleTrigger>
  );
}

function ToolFallbackContent({
  className,
  children,
  ...props
}: React.ComponentProps<typeof CollapsibleContent>) {
  return (
    <CollapsibleContent
      data-slot="tool-fallback-content"
      className={cn(
        'aui-tool-fallback-content relative overflow-hidden text-sm outline-none',
        'group/collapsible-content ease-[cubic-bezier(0.32,0.72,0,1)] motion-reduce:animate-none',
        'data-[state=closed]:animate-collapsible-up',
        'data-[state=open]:animate-collapsible-down',
        'data-[state=closed]:fill-mode-forwards',
        'data-[state=closed]:pointer-events-none',
        '[--tw-duration:var(--animation-duration)]',
        className
      )}
      {...props}>
      <div
        className={cn(
          'flex flex-col gap-2 ps-6 pt-1 pb-2 ease-[cubic-bezier(0.32,0.72,0,1)] motion-reduce:animate-none',
          'group-data-open/collapsible-content:animate-in group-data-open/collapsible-content:fade-in-0 group-data-open/collapsible-content:blur-in-[2px] group-data-open/collapsible-content:slide-in-from-top-1',
          'group-data-closed/collapsible-content:animate-out group-data-closed/collapsible-content:fade-out-0 group-data-closed/collapsible-content:blur-out-[2px] group-data-closed/collapsible-content:slide-out-to-top-1',
          'group-data-closed/collapsible-content:animation-duration-(--animation-duration) group-data-open/collapsible-content:animation-duration-(--animation-duration)'
        )}>
        {children}
      </div>
    </CollapsibleContent>
  );
}

function ToolFallbackArgs({
  argsText,
  className,
  ...props
}: React.ComponentProps<'div'> & { argsText?: string }) {
  if (!argsText) return null;

  return (
    <div
      data-slot="tool-fallback-args"
      className={cn('aui-tool-fallback-args', className)}
      {...props}>
      <pre className="aui-tool-fallback-args-value bg-muted/50 text-foreground/90 rounded-md p-2.5 text-xs break-words whitespace-pre-wrap">
        {argsText}
      </pre>
    </div>
  );
}

const formatUnknownValue = (value: unknown, space?: number): string => {
  if (typeof value === 'string') return value;

  try {
    if (value instanceof Error) return String(value);

    const json = JSON.stringify(value, null, space);
    if (json !== undefined) return json;
  } catch {
    // fall through to the String() fallback below
  }

  try {
    return String(value);
  } catch {
    return '[Unserializable value]';
  }
};

function ToolFallbackResult({
  result,
  className,
  ...props
}: React.ComponentProps<'div'> & { result?: unknown }) {
  const { t } = useT();
  if (result === undefined) return null;

  return (
    <div
      data-slot="tool-fallback-result"
      className={cn('aui-tool-fallback-result', className)}
      {...props}>
      <p className="aui-tool-fallback-result-header text-muted-foreground text-xs font-medium">
        {t('toolFallback.result', 'Result:')}
      </p>
      <pre className="aui-tool-fallback-result-content bg-muted/50 text-foreground/90 mt-1 rounded-md p-2.5 text-xs break-words whitespace-pre-wrap">
        {formatUnknownValue(result, 2)}
      </pre>
    </div>
  );
}

function ToolFallbackError({
  status,
  className,
  ...props
}: React.ComponentProps<'div'> & { status?: ToolCallMessagePartStatus }) {
  if (status?.type !== 'incomplete') return null;

  const error = status.error;
  const errorText = error === undefined || error === null ? null : formatUnknownValue(error);

  if (!errorText) return null;

  const isCancelled = status.reason === 'cancelled';
  const headerText = isCancelled ? 'Cancelled reason:' : 'Error:';

  return (
    <div
      data-slot="tool-fallback-error"
      className={cn('aui-tool-fallback-error', className)}
      {...props}>
      <p className="aui-tool-fallback-error-header text-muted-foreground font-semibold">
        {headerText}
      </p>
      <p className="aui-tool-fallback-error-reason text-muted-foreground">{errorText}</p>
    </div>
  );
}

const APPROVED_RESULT = 'Approved by user';
const DENIED_RESULT = 'User denied tool execution';

const APPROVAL_OPTION_DEFAULT_LABELS: Record<string, string> = {
  'allow-once': 'Allow',
  'allow-always': 'Always allow',
  'reject-once': 'Deny',
  'reject-always': 'Always deny',
};

const isKnownKind = (kind: string) => Object.hasOwn(APPROVAL_OPTION_DEFAULT_LABELS, kind);

const isAllowKind = (kind: string) => kind === 'allow-once' || kind === 'allow-always';

const approvalOptionLabel = (option: ToolApprovalOption) =>
  option.label ??
  (isKnownKind(option.kind) ? APPROVAL_OPTION_DEFAULT_LABELS[option.kind] : undefined) ??
  option.id;

/**
 * A request that declares how it wants to be presented is asking a question,
 * not gating an action, so a refusal is not one of the answers it accepts
 * unless the request declares itself dismissible.
 */
const isQuestion = (approval: ToolCallMessagePart['approval']) =>
  approval?.display === 'select' || approval?.display === 'text';

const offersInterruptAction = (
  status: ToolCallMessagePartStatus | undefined,
  approval: ToolCallMessagePart['approval'],
  interrupt: ToolCallMessagePart['interrupt']
) =>
  status?.type !== 'requires-action' ||
  status.reason !== 'interrupt' ||
  approval != null ||
  interrupt != null;

function ToolFallbackApproval({
  className,
  addResult,
  resume,
  interrupt,
  approval,
  respondToApproval,
  status,
  ...props
}: React.ComponentProps<'div'> &
  Partial<
    Pick<ToolCallMessagePartProps, 'addResult' | 'resume' | 'respondToApproval' | 'status'>
  > & {
    interrupt?: ToolCallMessagePart['interrupt'];
    approval?: ToolCallMessagePart['approval'];
  }) {
  const { t } = useT();
  const [submitted, setSubmitted] = useState(false);
  const voiceActive = useAuiState(s => s.thread.voice !== undefined);
  const locked = submitted || voiceActive;
  const [confirmingId, setConfirmingId] = useState<string | null>(null);
  const [answer, setAnswer] = useState('');
  const [error, setError] = useState<string | null>(null);

  if (approval != null && (approval.approved !== undefined || approval.resolution !== undefined))
    return null;

  if (!offersInterruptAction(status, approval, interrupt)) return null;

  // A declared option list is a host constraint: the kit never adds an
  // approval path beyond it, and preserves a refusal path only where the
  // request is an action the user may refuse.
  const declaredOptions = respondToApproval ? approval?.options : undefined;
  const acceptsText =
    approval != null && respondToApproval != null && toolApprovalAcceptsText(approval);

  // A refused response leaves the request open, so the controls come back
  // rather than staying spent on a decision the runtime never recorded.
  const submit = (send: () => Promise<void> | void) => {
    setSubmitted(true);
    setError(null);
    void (async () => {
      try {
        await send();
      } catch (sendError) {
        setSubmitted(false);
        setError(sendError instanceof Error ? sendError.message : String(sendError));
      }
    })();
  };

  const typedNote = () => (answer.trim() ? { text: answer } : {});

  const respond = (approved: boolean) => {
    if (locked) return;
    if (approval != null && approval.approved === undefined && respondToApproval) {
      submit(() => respondToApproval({ approved, ...typedNote() }));
    } else if (interrupt) {
      submit(() => resume?.({ approved }));
    } else if (status?.type === 'requires-action' && status.reason === 'interrupt') {
      return;
    } else {
      submit(() => addResult?.(approved ? APPROVED_RESULT : DENIED_RESULT));
    }
  };

  const respondWithOption = (option: ToolApprovalOption) => {
    if (locked) return;
    setConfirmingId(null);
    // A custom kind has no decision class for the runtime to derive, and
    // responding without one throws; picking a declared option is an answer,
    // so it resolves as approved.
    submit(() =>
      respondToApproval?.(
        isKnownKind(option.kind)
          ? { optionId: option.id, ...typedNote() }
          : { optionId: option.id, approved: true, ...typedNote() }
      )
    );
  };

  // The kit does not validate an answer the request never constrained: a host
  // that cannot record an empty one rejects it, which reopens the controls.
  const submitAnswer = () => {
    if (locked) return;
    submit(() => respondToApproval?.({ text: answer }));
  };

  const handleOption = (option: ToolApprovalOption) => {
    if (option.confirm) {
      setConfirmingId(option.id);
    } else {
      respondWithOption(option);
    }
  };

  const confirming =
    confirmingId != null ? declaredOptions?.find(o => o.id === confirmingId) : undefined;

  const question = isQuestion(approval);
  // `approval.dismissible` is not on the `@assistant-ui/core` 0.3.20 type
  // pinned here (only `id`/`prompt`/`display`/`allowFreeform`/decision fields
  // are) — a further version bump is needed before a question can offer a
  // "Dismiss" affordance; until then a question-mode request always renders
  // its full answer surface.
  const dismissButton = null;

  const promptText = approval?.prompt ? (
    <p className="aui-tool-fallback-approval-prompt text-foreground whitespace-pre-line">
      {approval.prompt}
    </p>
  ) : null;

  const errorText = error ? (
    <p
      role="alert"
      className="aui-tool-fallback-approval-error text-destructive text-xs whitespace-pre-line">
      {error}
    </p>
  ) : null;

  const answerField = acceptsText ? (
    <div className="aui-tool-fallback-approval-answer flex flex-col items-start gap-2">
      <Textarea
        value={answer}
        onChange={event => setAnswer(event.target.value)}
        disabled={locked}
        aria-label={question ? (approval?.prompt ?? 'Answer') : 'Note'}
        placeholder={question ? 'Type your answer' : 'Add a note to your decision'}
      />
      {question && (
        <div className="flex items-center gap-2">
          <Button size="sm" className={pressable} onClick={submitAnswer} disabled={locked}>
            {t('chat.elicitation.send', 'Send')}
          </Button>
          {dismissButton}
        </div>
      )}
    </div>
  ) : null;

  if (confirming) {
    const confirmMeta = typeof confirming.confirm === 'object' ? confirming.confirm : undefined;
    const confirmDescription = confirmMeta?.description ?? confirming.description;
    return (
      <div
        data-slot="tool-fallback-approval-confirm"
        className={cn('aui-tool-fallback-approval-confirm flex flex-col gap-2 pt-1', className)}
        {...props}>
        <p className="aui-tool-fallback-approval-confirm-title font-semibold">
          {confirmMeta?.title ?? `${approvalOptionLabel(confirming)}?`}
        </p>
        {confirmDescription && (
          <p className="aui-tool-fallback-approval-confirm-description text-muted-foreground whitespace-pre-line">
            {confirmDescription}
          </p>
        )}
        {confirming.grants && confirming.grants.length > 0 && (
          <ul className="aui-tool-fallback-approval-confirm-grants flex flex-col gap-1">
            {confirming.grants.map(grant => (
              <li key={grant}>
                <code className="aui-tool-fallback-approval-confirm-grant bg-muted rounded px-1.5 py-0.5 text-xs">
                  {grant}
                </code>
              </li>
            ))}
          </ul>
        )}
        <div className="flex items-center gap-2">
          <Button
            size="sm"
            className={pressable}
            onClick={() => respondWithOption(confirming)}
            disabled={locked}>
            {t('common.confirm', 'Confirm')}
          </Button>
          <Button
            size="sm"
            variant="outline"
            className={pressable}
            onClick={() => setConfirmingId(null)}
            disabled={locked}>
            {t('common.back', 'Back')}
          </Button>
        </div>
      </div>
    );
  }

  if (declaredOptions && declaredOptions.length > 0) {
    const allowOptions = declaredOptions.filter(o => isAllowKind(o.kind));
    const customOptions = declaredOptions.filter(o => !isKnownKind(o.kind));
    const rejectOptions = declaredOptions.filter(o => isKnownKind(o.kind) && !isAllowKind(o.kind));
    return (
      <div
        data-slot="tool-fallback-approval"
        className={cn('aui-tool-fallback-approval flex flex-col gap-2 pt-1', className)}
        {...props}>
        {promptText}
        <div className="flex flex-wrap items-center gap-2">
          {[...allowOptions, ...customOptions, ...rejectOptions].map(option => (
            <Button
              key={option.id}
              size="sm"
              variant={option === allowOptions[0] ? 'default' : 'outline'}
              className={pressable}
              onClick={() => handleOption(option)}
              disabled={locked}>
              {approvalOptionLabel(option)}
            </Button>
          ))}
          {rejectOptions.length === 0 && !question && (
            <Button
              size="sm"
              variant="outline"
              className={pressable}
              onClick={() => respond(false)}
              disabled={locked}>
              {t('chat.approval.deny', 'Deny')}
            </Button>
          )}
          {!acceptsText && dismissButton}
        </div>
        {answerField}
        {errorText}
      </div>
    );
  }

  // A question carries no decision to fabricate, so it renders only what the
  // request declared, even when that leaves nothing to act on here.
  if (question) {
    return (
      <div
        data-slot="tool-fallback-approval"
        className={cn('aui-tool-fallback-approval flex flex-col gap-2 pt-1', className)}
        {...props}>
        {promptText}
        {answerField}
        {!acceptsText && dismissButton && (
          <div className="flex items-center gap-2">{dismissButton}</div>
        )}
        {errorText}
      </div>
    );
  }

  return (
    <div
      data-slot="tool-fallback-approval"
      className={cn('aui-tool-fallback-approval flex flex-col gap-2 pt-1', className)}
      {...props}>
      {promptText}
      <div className="flex items-center gap-2">
        <Button size="sm" className={pressable} onClick={() => respond(true)} disabled={locked}>
          {t('workflows.phase.toolScope.allow', 'Allow')}
        </Button>
        <Button
          size="sm"
          variant="outline"
          className={pressable}
          onClick={() => respond(false)}
          disabled={locked}>
          {t('chat.approval.deny', 'Deny')}
        </Button>
      </div>
      {answerField}
      {errorText}
    </div>
  );
}

const ToolFallbackImpl: ToolCallMessagePartComponent = ({
  toolName,
  argsText,
  result,
  status,
  addResult,
  resume,
  interrupt,
  approval,
  respondToApproval,
}) => {
  const isCancelled = status?.type === 'incomplete' && status.reason === 'cancelled';
  const isRequiresAction = status?.type === 'requires-action';
  const shouldRenderApproval =
    isRequiresAction && offersInterruptAction(status, approval, interrupt);

  const [open, setOpen] = useState(isRequiresAction);
  const [prevRequiresAction, setPrevRequiresAction] = useState(isRequiresAction);
  if (isRequiresAction !== prevRequiresAction) {
    setPrevRequiresAction(isRequiresAction);
    if (isRequiresAction) setOpen(true);
  }

  return (
    <ToolFallbackRoot open={open} onOpenChange={setOpen}>
      <ToolFallbackTrigger toolName={toolName} status={status} />
      <ToolFallbackContent>
        <ToolFallbackError status={status} />
        <ToolFallbackArgs argsText={argsText} className={cn(isCancelled && 'opacity-60')} />
        {shouldRenderApproval && (
          <ToolFallbackApproval
            addResult={addResult}
            resume={resume}
            interrupt={interrupt}
            approval={approval}
            respondToApproval={respondToApproval}
            status={status}
          />
        )}
        {!isCancelled && <ToolFallbackResult result={result} />}
      </ToolFallbackContent>
    </ToolFallbackRoot>
  );
};

const ToolFallback = memo(ToolFallbackImpl) as unknown as ToolCallMessagePartComponent & {
  Root: typeof ToolFallbackRoot;
  Trigger: typeof ToolFallbackTrigger;
  Content: typeof ToolFallbackContent;
  Args: typeof ToolFallbackArgs;
  Result: typeof ToolFallbackResult;
  Error: typeof ToolFallbackError;
  Approval: typeof ToolFallbackApproval;
};

ToolFallback.displayName = 'ToolFallback';
ToolFallback.Root = ToolFallbackRoot;
ToolFallback.Trigger = ToolFallbackTrigger;
ToolFallback.Content = ToolFallbackContent;
ToolFallback.Args = ToolFallbackArgs;
ToolFallback.Result = ToolFallbackResult;
ToolFallback.Error = ToolFallbackError;
ToolFallback.Approval = ToolFallbackApproval;

export {
  formatUnknownValue,
  offersInterruptAction,
  ToolFallback,
  ToolFallbackError,
  ToolFallbackApproval,
};
