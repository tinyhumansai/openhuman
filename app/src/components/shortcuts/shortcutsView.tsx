import { useMemo, useSyncExternalStore } from 'react';

import { GROUP_LABEL_KEYS } from '../../lib/commands/globalActions';
import { hotkeyManager } from '../../lib/commands/hotkeyManager';
import { registry } from '../../lib/commands/registry';
import type { RegisteredAction } from '../../lib/commands/types';
import { useT } from '../../lib/i18n/I18nContext';
import Kbd from '../commands/Kbd';

// Display order for the grouped shortcut list. Mirrors the command palette's
// ordering and appends Help. Unknown groups sort alphabetically after these.
const DISPLAY_ORDER = ['Navigation', 'Profiles', 'Chat', 'View', 'General', 'Help'];

export interface ShortcutGroup {
  group: string;
  items: RegisteredAction[];
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

/**
 * Live-grouped view of every currently-active action that has a keyboard
 * shortcut. Driven by the same registry the command palette uses, so the help
 * directory can never drift from the real bindings.
 */
export function useActiveShortcutGroups(): ShortcutGroup[] {
  const actions = useSyncExternalStore(subscribe, getSnapshot);
  return useMemo(() => {
    const byGroup = new Map<string, RegisteredAction[]>();
    for (const a of actions) {
      if (!a.shortcut) continue;
      const g = a.group ?? 'General';
      if (!byGroup.has(g)) byGroup.set(g, []);
      byGroup.get(g)!.push(a);
    }
    const keys = [...byGroup.keys()].sort((a, b) => {
      const ai = DISPLAY_ORDER.indexOf(a);
      const bi = DISPLAY_ORDER.indexOf(b);
      if (ai === -1 && bi === -1) return a.localeCompare(b);
      if (ai === -1) return 1;
      if (bi === -1) return -1;
      return ai - bi;
    });
    return keys.map(group => ({ group, items: byGroup.get(group)! }));
  }, [actions]);
}

interface ShortcutsListProps {
  /** Visual density. `modal` is compact; `panel` matches settings cards. */
  variant?: 'modal' | 'panel';
}

/**
 * Renders the grouped, live shortcut list. Shared by the `?`/⌘/ help overlay
 * and the Settings → Keyboard Shortcuts panel so they never disagree.
 */
export function ShortcutsList({ variant = 'modal' }: ShortcutsListProps) {
  const { t } = useT();
  const groups = useActiveShortcutGroups();

  if (groups.length === 0) {
    return (
      <p className="px-1 py-6 text-center text-sm text-stone-500 dark:text-neutral-400">
        {t('shortcuts.empty')}
      </p>
    );
  }

  const rowClass =
    variant === 'panel'
      ? 'flex items-center justify-between gap-4 px-4 py-3'
      : 'flex items-center justify-between gap-4 px-1 py-1.5';

  return (
    <div className={variant === 'panel' ? 'space-y-5' : 'space-y-4'}>
      {groups.map(({ group, items }) => (
        <section key={group}>
          <h3 className="px-1 pb-1 text-xs font-semibold uppercase tracking-wide text-stone-400 dark:text-neutral-500">
            {GROUP_LABEL_KEYS[group] ? t(GROUP_LABEL_KEYS[group]) : group}
          </h3>
          <div
            className={
              variant === 'panel'
                ? 'overflow-hidden rounded-xl border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 divide-y divide-neutral-100 dark:divide-neutral-800'
                : 'divide-y divide-stone-100 dark:divide-neutral-800'
            }>
            {items.map(action => (
              <div key={action.id} className={rowClass}>
                <span className="min-w-0 flex items-center gap-3 truncate text-sm text-stone-700 dark:text-neutral-200">
                  {action.icon ? (
                    <action.icon className="w-4 h-4 flex-shrink-0 text-stone-400 dark:text-neutral-500" />
                  ) : null}
                  <span className="truncate">
                    {action.labelKey ? t(action.labelKey) : action.label}
                  </span>
                </span>
                {action.shortcut ? <Kbd shortcut={action.shortcut} size="md" /> : null}
              </div>
            ))}
          </div>
        </section>
      ))}
    </div>
  );
}
