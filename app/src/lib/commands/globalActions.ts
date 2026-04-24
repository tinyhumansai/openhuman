import type { NavigateFunction } from 'react-router-dom';

import { hotkeyManager } from './hotkeyManager';
import { registry } from './registry';

export const GROUP_ORDER = ['Navigation'] as const;

export function registerGlobalActions(
  navigate: NavigateFunction,
  globalScopeSymbol: symbol
): () => void {
  const nav = (path: string) => () => {
    navigate(path);
  };

  const actions = [
    {
      id: 'nav.home',
      label: 'Go Home',
      group: 'Navigation',
      shortcut: 'mod+1',
      handler: nav('/home'),
      keywords: ['dashboard'],
    },
    {
      id: 'nav.chat',
      label: 'Go to Chat',
      group: 'Navigation',
      shortcut: 'mod+2',
      handler: nav('/chat'),
      keywords: ['conversations', 'messages', 'inbox'],
    },
    {
      id: 'nav.intelligence',
      label: 'Go to Intelligence',
      group: 'Navigation',
      shortcut: 'mod+3',
      handler: nav('/intelligence'),
      keywords: ['memory', 'knowledge'],
    },
    {
      id: 'nav.skills',
      label: 'Go to Skills',
      group: 'Navigation',
      shortcut: 'mod+4',
      handler: nav('/skills'),
      keywords: ['plugins', 'tools'],
    },
    {
      id: 'nav.settings',
      label: 'Open Settings',
      group: 'Navigation',
      shortcut: 'mod+,',
      handler: nav('/settings'),
      keywords: ['preferences', 'config'],
    },
  ];

  const disposers: Array<() => void> = [];
  for (const a of actions) {
    const disposeRegistry = registry.registerAction(a, globalScopeSymbol);
    const bindingSym = hotkeyManager.bind(globalScopeSymbol, {
      shortcut: a.shortcut,
      handler: a.handler,
      id: a.id,
    });
    disposers.push(() => {
      disposeRegistry();
      hotkeyManager.unbind(globalScopeSymbol, bindingSym);
    });
  }

  return () => {
    for (const d of disposers) d();
  };
}
