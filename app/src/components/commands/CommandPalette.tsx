import * as Dialog from '@radix-ui/react-dialog';
import { Command } from 'cmdk';
import { useMemo, useSyncExternalStore } from 'react';

import { hotkeyManager } from '../../lib/commands/hotkeyManager';
import { registry } from '../../lib/commands/registry';
import type { RegisteredAction } from '../../lib/commands/types';
import Kbd from './Kbd';

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

function subscribe(listener: () => void): () => void {
  const u1 = registry.subscribe(listener);
  const u2 = hotkeyManager.subscribe(listener);
  return () => {
    u1();
    u2();
  };
}

function getSnapshot(): RegisteredAction[] {
  return registry.getActiveActions(hotkeyManager.getStackSymbols());
}

export default function CommandPalette({ open, onOpenChange }: Props) {
  const actions = useSyncExternalStore(subscribe, getSnapshot);

  const groups = useMemo(() => {
    const byGroup = new Map<string, RegisteredAction[]>();
    for (const a of actions) {
      const g = a.group ?? 'Actions';
      if (!byGroup.has(g)) byGroup.set(g, []);
      byGroup.get(g)!.push(a);
    }
    const order = ['Navigation', 'Help'];
    const keys = [...byGroup.keys()].sort((a, b) => {
      const ai = order.indexOf(a);
      const bi = order.indexOf(b);
      if (ai === -1 && bi === -1) return a.localeCompare(b);
      if (ai === -1) return 1;
      if (bi === -1) return -1;
      return ai - bi;
    });
    return keys.map(k => [k, byGroup.get(k)!] as const);
  }, [actions]);

  function runAction(action: RegisteredAction): void {
    onOpenChange(false);
    window.requestAnimationFrame(() => {
      registry.runAction(action.id);
    });
  }

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 bg-cmd-overlay z-40" />
        <Dialog.Content
          className="fixed left-1/2 top-[20vh] -translate-x-1/2 w-[min(640px,calc(100vw-32px))] bg-cmd-surface text-cmd-foreground border border-cmd-border rounded-xl shadow-cmd-palette z-50 overflow-hidden"
          aria-label="Command palette">
          <Dialog.Title className="sr-only">Command palette</Dialog.Title>
          <Dialog.Description className="sr-only">
            Search and run commands. Use arrow keys to navigate, Enter to select, Escape to close.
          </Dialog.Description>
          <Command label="Commands" shouldFilter={true}>
            <Command.Input
              autoFocus
              placeholder="Type a command or search…"
              className="w-full px-4 py-3 bg-transparent outline-none border-b border-cmd-border text-cmd-foreground placeholder:text-cmd-foreground-muted"
              aria-label="Search commands"
            />
            <Command.List className="max-h-[50vh] overflow-auto py-2">
              <Command.Empty className="px-4 py-8 text-center text-cmd-foreground-muted">
                No results.
              </Command.Empty>
              {groups.map(([groupName, items]) => (
                <Command.Group
                  key={groupName}
                  heading={groupName}
                  className="[&_[cmdk-group-heading]]:px-4 [&_[cmdk-group-heading]]:py-1 [&_[cmdk-group-heading]]:text-xs [&_[cmdk-group-heading]]:uppercase [&_[cmdk-group-heading]]:text-cmd-foreground-muted">
                  {items.map(action => (
                    <Command.Item
                      key={action.id}
                      value={action.id}
                      keywords={[action.label, ...(action.keywords ?? [])]}
                      onSelect={() => runAction(action)}
                      className="flex items-center gap-3 px-4 py-2 cursor-pointer aria-selected:bg-cmd-surface-elevated">
                      {action.icon ? (
                        <action.icon className="w-4 h-4 text-cmd-foreground-muted" />
                      ) : (
                        <span className="w-4" />
                      )}
                      <span className="flex-1 truncate">{action.label}</span>
                      {action.hint && (
                        <span className="text-xs text-cmd-foreground-muted truncate">
                          {action.hint}
                        </span>
                      )}
                      {action.shortcut && <Kbd shortcut={action.shortcut} />}
                    </Command.Item>
                  ))}
                </Command.Group>
              ))}
            </Command.List>
            <div className="px-4 py-2 border-t border-cmd-border text-xs text-cmd-foreground-muted">
              Press ? for all shortcuts
            </div>
          </Command>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
