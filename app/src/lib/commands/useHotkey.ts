import { useContext, useEffect, useRef } from 'react';

import { hotkeyManager } from './hotkeyManager';
import { ScopeContext } from './ScopeContext';
import type { HotkeyBinding } from './types';

type HotkeyOptions = Omit<HotkeyBinding, 'shortcut' | 'handler'>;

export function useHotkey(
  shortcut: string,
  handler: () => void,
  options: HotkeyOptions = {}
): void {
  const frame = useContext(ScopeContext);
  const handlerRef = useRef(handler);
  const optsRef = useRef(options);
  handlerRef.current = handler;
  optsRef.current = options;

  useEffect(() => {
    const stable = () => handlerRef.current();
    // Always route `enabled` through the ref; callers can toggle it at any
    // render without rebinding.
    const stableEnabled = () => optsRef.current.enabled?.() ?? true;
    const sym = hotkeyManager.bind(frame, {
      shortcut,
      handler: stable,
      allowInInput: optsRef.current.allowInInput,
      repeat: optsRef.current.repeat,
      preventDefault: optsRef.current.preventDefault,
      enabled: stableEnabled,
      description: optsRef.current.description,
      id: optsRef.current.id,
    });
    return () => hotkeyManager.unbind(frame, sym);
  }, [shortcut, frame]);
}
