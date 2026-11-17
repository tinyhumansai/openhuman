'use client';

/**
 * assistant-ui's permission-grant element: a capability request (e.g. an
 * OAuth connect) with a "this grants" list and Deny / This session / Always
 * decisions.
 *
 * Vendored from the assistant-ui `elements-permission-grant` registry item
 * (https://r.assistant-ui.com/styles/base-nova/elements-permission-grant.json).
 * Changes from upstream:
 * - `cn` import path (`@/components/assistant-ui/lib/utils`).
 * - Every user-facing label is now a prop with an English default, so the
 *   calling adapter supplies the translated string via `useT()`.
 * - `denyProps` / `sessionProps` / `alwaysProps` slots, same rationale as
 *   `approval-card.tsx`: OpenHuman's e2e specs click a decision by its
 *   `data-analytics-id`, which the upstream buttons carry no way to attach.
 * - `pendingLabel` / `deniedLabel` / `grantedLabel` are now format functions
 *   (`(scope) => string`) rather than a bare string, so the OpenHuman connect
 *   card can render "granted · always" from one translated template
 *   (`chat.approval.*` keys use `{scope}` placeholders) instead of
 *   concatenating English words.
 * - Added a `busy` state distinct from `pending`: the OpenHuman OAuth handoff
 *   (poll for connection) has an in-flight phase with no decision buttons yet
 *   resolved, which upstream's `pending`/`GrantScope` union does not model.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import { KeyRoundIcon } from 'lucide-react';
import type { ComponentProps } from 'react';

import { field, inkButton, mono, paper } from './surfaces';

/**
 * A button-slot prop type that permits `data-analytics-id` / `data-testid`
 * (and any other `data-*` attribute) on an object literal. Plain
 * `ComponentProps<'button'>` fails TypeScript's excess-property check for a
 * literal assigned to a typed prop (JSX itself allows any `data-*`
 * attribute; a plain object literal does not inherit that allowance).
 */
type ButtonSlotProps = ComponentProps<'button'> & Record<`data-${string}`, string>;

export type GrantScope = 'session' | 'always' | 'denied';

export function PermissionGrant({
  capability,
  requester,
  requesterLabel = 'requested by',
  reach,
  reachLabel = 'this grants',
  scope,
  onGrant,
  denyLabel = 'Deny',
  sessionLabel = 'This session',
  alwaysLabel = 'Always',
  pendingLabel = 'pending',
  deniedLabel = 'denied',
  grantedLabel = (grantedScope: GrantScope) => `granted · ${grantedScope}`,
  denyProps,
  sessionProps,
  alwaysProps,
  className,
  ...props
}: Omit<
  ComponentProps<'div'>,
  'children' | 'capability' | 'requester' | 'reach' | 'scope' | 'onGrant'
> & {
  capability: string;
  requester: string;
  requesterLabel?: string;
  reach: readonly string[];
  reachLabel?: string;
  scope: GrantScope | 'pending' | 'busy';
  onGrant?: (scope: GrantScope) => void;
  denyLabel?: string;
  sessionLabel?: string;
  alwaysLabel?: string;
  pendingLabel?: string;
  deniedLabel?: string;
  grantedLabel?: (scope: GrantScope) => string;
  denyProps?: ButtonSlotProps;
  sessionProps?: ButtonSlotProps;
  alwaysProps?: ButtonSlotProps;
}) {
  return (
    <div
      data-slot="permission-grant"
      className={cn(paper, 'flex w-full flex-col gap-3.5 rounded-[20px] p-4', className)}
      {...props}>
      <div className="flex items-center gap-2.5">
        <span className="bg-foreground/[0.05] text-muted-foreground flex size-7 shrink-0 items-center justify-center rounded-lg">
          <KeyRoundIcon className="size-3.5" />
        </span>
        <div className="flex min-w-0 flex-1 flex-col">
          <span className="truncate text-[13.5px] font-medium">{capability}</span>
          <span className="text-muted-foreground truncate text-xs">
            {requesterLabel} {requester}
          </span>
        </div>
      </div>

      <div className="flex flex-col gap-1">
        <span className={cn(mono, 'text-muted-foreground')}>{reachLabel}</span>
        {reach.map(item => (
          <span key={item} className="text-muted-foreground flex items-baseline gap-2 text-xs">
            <span aria-hidden className="bg-foreground/20 size-1 rounded-full" />
            {item}
          </span>
        ))}
      </div>

      <div className="flex h-8 items-center justify-end gap-2">
        {scope === 'pending' || scope === 'busy' ? (
          onGrant && scope === 'pending' ? (
            <>
              <button
                type="button"
                onClick={() => onGrant('denied')}
                {...denyProps}
                className={cn(
                  'text-muted-foreground hover:bg-foreground/[0.06] hover:text-foreground/90 h-8 rounded-full px-3 text-xs font-medium transition-[background-color,color,scale] duration-150 active:scale-[0.96]',
                  denyProps?.className
                )}>
                {denyLabel}
              </button>
              <button
                type="button"
                onClick={() => onGrant('session')}
                {...sessionProps}
                className={cn(
                  'text-muted-foreground hover:bg-foreground/[0.06] hover:text-foreground/90 h-8 rounded-full px-3 text-xs font-medium transition-[background-color,color,scale] duration-150 active:scale-[0.96]',
                  sessionProps?.className
                )}>
                {sessionLabel}
              </button>
              <button
                type="button"
                onClick={() => onGrant('always')}
                {...alwaysProps}
                className={cn(
                  inkButton,
                  'flex h-8 items-center rounded-full px-3 text-xs font-medium',
                  alwaysProps?.className
                )}>
                {alwaysLabel}
              </button>
            </>
          ) : (
            <span
              key={scope}
              className={cn(
                field,
                mono,
                'fade-in animate-in text-muted-foreground rounded-full px-2.5 py-1.5 duration-300'
              )}>
              {pendingLabel}
            </span>
          )
        ) : (
          <span
            key={scope}
            className={cn(
              field,
              mono,
              'fade-in animate-in text-muted-foreground rounded-full px-2.5 py-1.5 duration-300'
            )}>
            {scope === 'denied' ? deniedLabel : grantedLabel(scope)}
          </span>
        )}
      </div>
    </div>
  );
}
