'use client';

/**
 * assistant-ui's approval-card element: a decision surface for a parked tool
 * call (or any other approve/deny prompt), with the exact command/target
 * shown above Deny / Always allow / Allow once.
 *
 * Vendored from the assistant-ui `elements-approval-card` registry item
 * (https://r.assistant-ui.com/styles/base-nova/elements-approval-card.json).
 * Changes from upstream:
 * - `cn` import path (`@/components/assistant-ui/lib/utils`, this app's alias).
 * - Every user-facing label (`Deny`, `Always allow`, `Allow once`, `Approved,
 *   running`, `Denied`, `Finished with exit 0`) is now a prop with an English
 *   default, so the calling adapter supplies the translated string via
 *   `useT()` instead of the label being hard-coded.
 * - `allowOnceProps` / `alwaysAllowProps` / `denyProps` slots: OpenHuman's
 *   WDIO/Playwright specs click a decision button by its
 *   `data-analytics-id` (`agent-harness-behaviors.spec.ts`), and the upstream
 *   buttons carry no per-button identity — only the root div spreads
 *   `...props`. These optional `ComponentProps<'button'>` slots let a caller
 *   attach `data-analytics-id`/`data-testid` without duplicating the button.
 * - `expiry` slot: an optional node rendered next to the subtitle for a TTL
 *   countdown (`ApprovalRequestCard`'s parked-request TTL has no upstream
 *   equivalent).
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import { CheckIcon, Loader2Icon, TerminalIcon, XIcon } from 'lucide-react';
import type { ComponentProps, ReactNode } from 'react';

import { field, inkButton, paper } from './surfaces';

/**
 * A button-slot prop type that permits `data-analytics-id` / `data-testid`
 * (and any other `data-*` attribute) on an object literal. Plain
 * `ComponentProps<'button'>` fails TypeScript's excess-property check for a
 * literal assigned to a typed prop (JSX itself allows any `data-*`
 * attribute; a plain object literal does not inherit that allowance).
 */
type ButtonSlotProps = ComponentProps<'button'> & Record<`data-${string}`, string>;

export type ApprovalState = 'request' | 'running' | 'done' | 'denied';

export function ApprovalCard({
  state,
  command,
  title,
  subtitle,
  expiry,
  onAllowOnce,
  onAlwaysAllow,
  onDeny,
  denyLabel = 'Deny',
  alwaysAllowLabel = 'Always allow',
  allowOnceLabel = 'Allow once',
  runningLabel = 'Approved, running',
  deniedLabel = 'Denied',
  doneLabel = 'Finished with exit 0',
  allowOnceProps,
  alwaysAllowProps,
  denyProps,
  className,
  ...props
}: Omit<
  ComponentProps<'div'>,
  | 'children'
  | 'state'
  | 'command'
  | 'title'
  | 'subtitle'
  | 'onAllowOnce'
  | 'onAlwaysAllow'
  | 'onDeny'
> & {
  state: ApprovalState;
  command: string;
  title: string;
  subtitle: string;
  /** Optional node rendered beside the subtitle (e.g. a TTL countdown). */
  expiry?: ReactNode;
  onAllowOnce?: () => void;
  onAlwaysAllow?: () => void;
  onDeny?: () => void;
  denyLabel?: string;
  alwaysAllowLabel?: string;
  allowOnceLabel?: string;
  runningLabel?: string;
  deniedLabel?: string;
  doneLabel?: string;
  allowOnceProps?: ButtonSlotProps;
  alwaysAllowProps?: ButtonSlotProps;
  denyProps?: ButtonSlotProps;
}) {
  return (
    <div
      data-slot="approval-card"
      className={cn(paper, 'flex w-full flex-col gap-3.5 rounded-[20px] p-4', className)}
      {...props}>
      <div className="flex items-center gap-3">
        <span className="bg-foreground/[0.05] text-muted-foreground flex size-9 shrink-0 items-center justify-center rounded-xl">
          <TerminalIcon className="size-4" />
        </span>
        <div className="flex flex-col">
          <p className="text-[13.5px] font-medium">{title}</p>
          <p className="text-muted-foreground text-xs">{subtitle}</p>
          {expiry}
        </div>
      </div>

      <div
        className={cn(field, 'text-muted-foreground rounded-xl px-3.5 py-2.5 font-mono text-xs')}>
        {command}
      </div>

      <div className="flex min-h-8 flex-wrap items-center justify-end gap-2">
        {state === 'request' ? (
          <>
            {onDeny && (
              <button
                type="button"
                onClick={onDeny}
                {...denyProps}
                className={cn(
                  'text-muted-foreground hover:bg-foreground/[0.06] hover:text-foreground/90 h-8 rounded-full px-3.5 text-xs font-medium transition-[background-color,color,scale] duration-150 active:scale-[0.96]',
                  denyProps?.className
                )}>
                {denyLabel}
              </button>
            )}
            {onAlwaysAllow && (
              <button
                type="button"
                onClick={onAlwaysAllow}
                {...alwaysAllowProps}
                className={cn(
                  'text-muted-foreground hover:bg-foreground/[0.06] hover:text-foreground/90 h-8 rounded-full px-3.5 text-xs font-medium transition-[background-color,color,scale] duration-150 active:scale-[0.96]',
                  alwaysAllowProps?.className
                )}>
                {alwaysAllowLabel}
              </button>
            )}
            {onAllowOnce && (
              <button
                type="button"
                onClick={onAllowOnce}
                {...allowOnceProps}
                className={cn(
                  inkButton,
                  'h-8 px-3.5 text-xs font-medium whitespace-nowrap',
                  allowOnceProps?.className
                )}>
                {allowOnceLabel}
              </button>
            )}
          </>
        ) : (
          <div
            key={state}
            className="fade-in animate-in text-muted-foreground flex items-center gap-2 text-xs duration-300 motion-reduce:animate-none">
            {state === 'running' ? (
              <>
                <Loader2Icon className="text-muted-foreground size-3.5 animate-spin motion-reduce:animate-none" />
                {runningLabel}
              </>
            ) : state === 'denied' ? (
              <>
                <XIcon className="text-muted-foreground size-3.5" />
                {deniedLabel}
              </>
            ) : (
              <>
                <CheckIcon className="size-3.5 text-emerald-500" />
                {doneLabel}
              </>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
