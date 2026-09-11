import type { Unstable_SlashCommand } from '@assistant-ui/react';
import { useMemo, useSyncExternalStore } from 'react';

import { useT } from '../i18n/I18nContext';
import { hotkeyManager } from './hotkeyManager';
import { registry } from './registry';
import type { RegisteredAction } from './types';

function subscribe(listener: () => void): () => void {
  const unregisterRegistry = registry.subscribe(listener);
  const unregisterHotkeys = hotkeyManager.subscribe(listener);
  return () => {
    unregisterRegistry();
    unregisterHotkeys();
  };
}

function getSnapshot(): RegisteredAction[] {
  return registry.getActiveActions(hotkeyManager.getStackSymbols());
}

export function slashCommandsFromActions(
  actions: readonly RegisteredAction[],
  translate: (key: string, fallback?: string) => string
): readonly Unstable_SlashCommand[] {
  return actions.flatMap(action => {
    const slash = action.slashCommand;
    if (!slash) return [];
    const descriptionKey = slash.descriptionKey ?? action.labelKey;
    return [
      {
        id: slash.id,
        description: descriptionKey ? translate(descriptionKey, action.label) : action.label,
        execute: () => {
          registry.runAction(action.id);
        },
      },
    ];
  });
}

/** Composer `/` commands projected from the same active registry as the palette. */
export function useSlashCommands(): readonly Unstable_SlashCommand[] {
  const { t } = useT();
  const actions = useSyncExternalStore(subscribe, getSnapshot);
  return useMemo(() => slashCommandsFromActions(actions, t), [actions, t]);
}
