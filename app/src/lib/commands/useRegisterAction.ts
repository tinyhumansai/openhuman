import { useContext, useEffect, useRef } from 'react';

import { hotkeyManager } from './hotkeyManager';
import { registry } from './registry';
import { ScopeContext } from './ScopeContext';
import { parseShortcut } from './shortcut';
import type { Action } from './types';

export function useRegisterAction(action: Action): void {
  const frame = useContext(ScopeContext);
  const handlerRef = useRef(action.handler);
  const enabledRef = useRef(action.enabled);
  handlerRef.current = action.handler;
  enabledRef.current = action.enabled;

  useEffect(() => {
    if (!frame) {
      throw new Error(
        'useRegisterAction: no ScopeContext frame. Wrap your tree in a ScopeProvider (e.g. CommandProvider).'
      );
    }
    const stable = () => {
      handlerRef.current();
    };
    // Always route enabled through the ref so flipping it between undefined
    // and a predicate takes effect without rebinding.
    const stableEnabled = () => enabledRef.current?.() ?? true;
    const disposeRegistry = registry.registerAction(
      { ...action, handler: stable, enabled: stableEnabled },
      frame
    );
    let bindingSym: symbol | undefined;
    if (action.shortcut) {
      parseShortcut(action.shortcut);
      bindingSym = hotkeyManager.bind(frame, {
        shortcut: action.shortcut,
        handler: stable,
        allowInInput: action.allowInInput,
        repeat: action.repeat,
        preventDefault: action.preventDefault,
        enabled: stableEnabled,
        id: action.id,
      });
    }
    return () => {
      disposeRegistry();
      if (bindingSym) hotkeyManager.unbind(frame, bindingSym);
    };
  }, [
    action.id,
    action.shortcut,
    action.allowInInput,
    action.repeat,
    action.preventDefault,
    frame,
  ]);
}
