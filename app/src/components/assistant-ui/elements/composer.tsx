'use client';

/**
 * The composer's `/` command menu and `@` mention menu: matching hooks and the
 * menu surface they render into.
 *
 * Vendored from the assistant-ui `elements-composer` registry item
 * (https://r.assistant-ui.com/styles/base-nova/elements-composer.json).
 * Changes from upstream:
 * - `cn` import path (`@/components/assistant-ui/lib/utils`).
 * - Only the slash-menu and mention pieces are vendored (`useSlashMatches`,
 *   `useMentionMatches`, `applyMention`, `ComposerMenu`, `ComposerMenuItem`,
 *   `ComposerCommandItem` and their types). The rest of
 *   the upstream file — attachments, voice, model picker, context ring and
 *   send button — is omitted: OpenHuman's composer renders those through
 *   `ComposerPrimitive` and Lexical in `thread.tsx`. The live product `/` and
 *   `@` pickers are the primitive-driven `composer-trigger-popover.tsx`, fed
 *   by `features/conversations/aui/useSlashCommandSource.ts` and
 *   `useMentionSource.ts`; this file renders the same fixtures in the dev
 *   gallery (`pages/dev/ToolCallGallery.tsx`).
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import type { LucideIcon } from 'lucide-react';
import { type ComponentProps, useMemo } from 'react';

import { field, floating } from './surfaces';

export interface ComposerCommand {
  name: string;
  description: string;
  icon: LucideIcon;
}

export interface ComposerPerson {
  name: string;
  role: 'agent' | 'human';
}

/** Commands whose name starts with the slash query, or none when not typing one. */
export function useSlashMatches(
  value: string,
  commands: readonly ComposerCommand[] | undefined
): ComposerCommand[] {
  return useMemo(() => {
    if (!commands || !value.startsWith('/')) return [];
    const query = value.slice(1).toLowerCase();
    return commands.filter(command => command.name.startsWith(query));
  }, [commands, value]);
}

/** People matching a trailing @mention, or none when the caret is not in one. */
export function useMentionMatches(
  value: string,
  people: readonly ComposerPerson[] | undefined
): ComposerPerson[] {
  return useMemo(() => {
    if (!people) return [];
    const match = /@([\w]*)$/.exec(value);
    if (!match) return [];
    const query = match[1]?.toLowerCase() ?? '';
    return people.filter(person => person.name.toLowerCase().startsWith(query));
  }, [people, value]);
}

/** Replaces the trailing @mention with the chosen name. */
export function applyMention(value: string, name: string): string {
  return value.replace(/@[\w]*$/, `@${name} `);
}

export function ComposerMenu({
  open,
  align = 'start',
  className,
  ...props
}: ComponentProps<'div'> & { open: boolean; align?: 'start' | 'end' }) {
  return (
    <div
      data-slot="composer-menu"
      data-open={open || undefined}
      className={cn(
        floating,
        'absolute bottom-full z-10 mb-2 flex w-72 flex-col gap-0.5 rounded-2xl p-1.5',
        align === 'start' ? 'start-0 origin-bottom-left' : 'end-0 origin-bottom-right',
        'transition-[opacity,scale] duration-200 ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none',
        open ? 'scale-100 opacity-100' : 'pointer-events-none scale-[0.97] opacity-0',
        className
      )}
      {...props}
    />
  );
}

export function ComposerMenuItem({
  active = false,
  className,
  ...props
}: ComponentProps<'button'> & { active?: boolean }) {
  return (
    <button
      type="button"
      data-slot="composer-menu-item"
      data-active={active || undefined}
      className={cn(
        'flex w-full items-center gap-2.5 rounded-[10px] px-2.5 py-2 text-[13.5px] transition-colors',
        active ? field : 'hover:bg-foreground/[0.04]',
        className
      )}
      {...props}
    />
  );
}

export function ComposerCommandItem({
  command,
  active,
  ...props
}: Omit<ComponentProps<'button'>, 'children'> & { command: ComposerCommand; active: boolean }) {
  return (
    <ComposerMenuItem active={active} {...props}>
      <command.icon className="text-muted-foreground size-3.5 shrink-0" />
      <span className="font-medium">/{command.name}</span>
      <span className="text-muted-foreground flex-1 truncate text-start text-xs">
        {command.description}
      </span>
      {active && (
        <kbd className="bg-foreground/[0.06] text-muted-foreground rounded px-1 font-mono text-[10px]">
          ↵
        </kbd>
      )}
    </ComposerMenuItem>
  );
}
