import { useEffect, useMemo, useState } from 'react';
import * as Dialog from '@radix-ui/react-dialog';
import { registry } from '../../lib/commands/registry';
import { hotkeyManager } from '../../lib/commands/hotkeyManager';
import Kbd from './Kbd';
import type { ActiveBinding, RegisteredAction } from '../../lib/commands/types';

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

function canonicalize(shortcut: string): string {
  return shortcut.toLowerCase().split('+').sort().join('+');
}

export default function HelpOverlay({ open, onOpenChange }: Props) {
  const [actions, setActions] = useState<RegisteredAction[]>(() =>
    registry.getActiveActions(hotkeyManager.getStackSymbols()).filter((a) => !!a.shortcut),
  );
  const [bindings, setBindings] = useState<ActiveBinding[]>(() =>
    hotkeyManager.getActiveBindings(),
  );

  useEffect(() => {
    const refresh = () => {
      setActions(
        registry
          .getActiveActions(hotkeyManager.getStackSymbols())
          .filter((a) => !!a.shortcut),
      );
      setBindings(hotkeyManager.getActiveBindings());
    };
    refresh();
    const u1 = registry.subscribe(refresh);
    const u2 = hotkeyManager.subscribe(refresh);
    return () => {
      u1();
      u2();
    };
  }, []);

  const { actionRows, shortcutRows } = useMemo(() => {
    const actionRows = [...actions].sort(
      (a, b) =>
        (a.group ?? '').localeCompare(b.group ?? '') || a.label.localeCompare(b.label),
    );
    const actionShortcutKeys = new Set(actions.map((a) => canonicalize(a.shortcut!)));
    const seen = new Set<string>();
    const shortcutRows: ActiveBinding[] = [];
    for (const b of bindings) {
      if (!b.binding.description) continue;
      const k = canonicalize(b.binding.shortcut);
      if (actionShortcutKeys.has(k) || seen.has(k)) continue;
      seen.add(k);
      shortcutRows.push(b);
    }
    return { actionRows, shortcutRows };
  }, [actions, bindings]);

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 bg-cmd-overlay z-40" />
        <Dialog.Content
          className="fixed left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 w-[min(560px,calc(100vw-32px))] max-h-[80vh] overflow-auto bg-cmd-surface text-cmd-foreground border border-cmd-border rounded-xl shadow-cmd-palette z-50 p-4"
          aria-label="Keyboard shortcuts"
        >
          <Dialog.Title className="sr-only">Keyboard help</Dialog.Title>
          {actionRows.length > 0 && (
            <section aria-label="Actions">
              <h3 className="text-xs uppercase text-cmd-foreground-muted mb-2">Actions</h3>
              <ul className="space-y-1">
                {actionRows.map((a) => (
                  <li key={a.id} className="flex items-center justify-between py-1">
                    <span>{a.label}</span>
                    <Kbd shortcut={a.shortcut!} />
                  </li>
                ))}
              </ul>
            </section>
          )}
          {shortcutRows.length > 0 && (
            <section aria-label="Shortcuts" className="mt-4">
              <h3 className="text-xs uppercase text-cmd-foreground-muted mb-2">Shortcuts</h3>
              <ul className="space-y-1">
                {shortcutRows.map((b, i) => (
                  <li key={i} className="flex items-center justify-between py-1">
                    <span>{b.binding.description}</span>
                    <Kbd shortcut={b.binding.shortcut} />
                  </li>
                ))}
              </ul>
            </section>
          )}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
