'use client';

import { cn } from '@/components/assistant-ui/lib/utils';
import { Button } from '@/components/assistant-ui/ui/button';
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/assistant-ui/ui/collapsible';
import {
  type ToolApprovalOption,
  type ToolCallMessagePart,
  type ToolCallMessagePartComponent,
  type ToolCallMessagePartProps,
  type ToolCallMessagePartStatus,
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
        'aui-tool-fallback-trigger group/trigger text-muted-foreground hover:text-foreground flex w-fit origin-left items-center gap-2 py-1.5 text-sm transition-[color,scale] active:scale-[0.98]',
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
        'aui-tool-fallback-content relative overflow-hidden text-sm outline-hidden',
        'group/collapsible-content ease-[cubic-bezier(0.32,0.72,0,1)] motion-reduce:animate-none',
        'data-closed:animate-collapsible-up',
        'data-open:animate-collapsible-down',
        'data-closed:fill-mode-forwards',
        'data-closed:pointer-events-none',
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
      <pre className="aui-tool-fallback-args-value bg-muted/50 text-foreground/90 rounded-md p-2.5 text-xs whitespace-pre-wrap">
        {argsText}
      </pre>
    </div>
  );
}

function ToolFallbackResult({
  result,
  className,
  ...props
}: React.ComponentProps<'div'> & { result?: unknown }) {
  if (result === undefined) return null;

  return (
    <div
      data-slot="tool-fallback-result"
      className={cn('aui-tool-fallback-result', className)}
      {...props}>
      <p className="aui-tool-fallback-result-header text-muted-foreground text-xs font-medium">
        Result:
      </p>
      <pre className="aui-tool-fallback-result-content bg-muted/50 text-foreground/90 mt-1 rounded-md p-2.5 text-xs whitespace-pre-wrap">
        {typeof result === 'string' ? result : JSON.stringify(result, null, 2)}
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
  const errorText = error ? (typeof error === 'string' ? error : JSON.stringify(error)) : null;

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

const isAllowKind = (kind: string) => kind === 'allow-once' || kind === 'allow-always';

const approvalOptionLabel = (option: ToolApprovalOption) =>
  option.label ??
  (Object.hasOwn(APPROVAL_OPTION_DEFAULT_LABELS, option.kind)
    ? APPROVAL_OPTION_DEFAULT_LABELS[option.kind]
    : undefined) ??
  option.id;

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
  const [submitted, setSubmitted] = useState(false);
  const [confirmingId, setConfirmingId] = useState<string | null>(null);

  if (approval != null && (approval.approved !== undefined || approval.resolution !== undefined))
    return null;

  if (!offersInterruptAction(status, approval, interrupt)) return null;

  // Custom (`_`-prefixed) kinds cannot be resolved to a boolean by the kit;
  // hosts using custom kinds render their own bar. A declared option list is
  // a host constraint: the kit never adds an approval path beyond it, but
  // always preserves a refusal path.
  const declaredOptions = respondToApproval ? approval?.options : undefined;
  const options = declaredOptions?.filter(o =>
    Object.hasOwn(APPROVAL_OPTION_DEFAULT_LABELS, o.kind)
  );

  const respond = (approved: boolean) => {
    if (submitted) return;
    if (approval != null && approval.approved === undefined && respondToApproval) {
      respondToApproval({ approved });
    } else if (interrupt) {
      resume?.({ approved });
    } else if (status?.type === 'requires-action' && status.reason === 'interrupt') {
      return;
    } else {
      addResult?.(approved ? APPROVED_RESULT : DENIED_RESULT);
    }
    setSubmitted(true);
  };

  const respondWithOption = (option: ToolApprovalOption) => {
    if (submitted) return;
    respondToApproval?.({ optionId: option.id });
    setSubmitted(true);
    setConfirmingId(null);
  };

  const handleOption = (option: ToolApprovalOption) => {
    if (option.confirm) {
      setConfirmingId(option.id);
    } else {
      respondWithOption(option);
    }
  };

  const confirming = confirmingId != null ? options?.find(o => o.id === confirmingId) : undefined;

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
          <p className="aui-tool-fallback-approval-confirm-description text-muted-foreground">
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
            disabled={submitted}>
            Confirm
          </Button>
          <Button
            size="sm"
            variant="outline"
            className={pressable}
            onClick={() => setConfirmingId(null)}
            disabled={submitted}>
            Back
          </Button>
        </div>
      </div>
    );
  }

  if (declaredOptions && declaredOptions.length > 0) {
    const allowOptions = options?.filter(o => isAllowKind(o.kind)) ?? [];
    const rejectOptions = options?.filter(o => !isAllowKind(o.kind)) ?? [];
    return (
      <div
        data-slot="tool-fallback-approval"
        className={cn(
          'aui-tool-fallback-approval flex flex-wrap items-center gap-2 pt-1',
          className
        )}
        {...props}>
        {[...allowOptions, ...rejectOptions].map(option => (
          <Button
            key={option.id}
            size="sm"
            variant={option === allowOptions[0] ? 'default' : 'outline'}
            className={pressable}
            onClick={() => handleOption(option)}
            disabled={submitted}>
            {approvalOptionLabel(option)}
          </Button>
        ))}
        {rejectOptions.length === 0 && (
          <Button
            size="sm"
            variant="outline"
            className={pressable}
            onClick={() => respond(false)}
            disabled={submitted}>
            Deny
          </Button>
        )}
      </div>
    );
  }

  return (
    <div
      data-slot="tool-fallback-approval"
      className={cn('aui-tool-fallback-approval flex items-center gap-2 pt-1', className)}
      {...props}>
      <Button size="sm" className={pressable} onClick={() => respond(true)} disabled={submitted}>
        Allow
      </Button>
      <Button
        size="sm"
        variant="outline"
        className={pressable}
        onClick={() => respond(false)}
        disabled={submitted}>
        Deny
      </Button>
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
  ToolFallback,
  ToolFallbackRoot,
  ToolFallbackTrigger,
  ToolFallbackContent,
  ToolFallbackArgs,
  ToolFallbackResult,
  ToolFallbackError,
  ToolFallbackApproval,
};
