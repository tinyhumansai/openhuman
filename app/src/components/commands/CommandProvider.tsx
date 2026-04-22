import { type ReactNode, useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { registerGlobalActions } from '../../lib/commands/globalActions';
import { hotkeyManager } from '../../lib/commands/hotkeyManager';
import { registry } from '../../lib/commands/registry';
import { ScopeContext } from '../../lib/commands/ScopeContext';
import CommandPalette from './CommandPalette';

let instanceCount = 0;

interface Props {
  children: ReactNode;
}

export default function CommandProvider({ children }: Props) {
  const navigate = useNavigate();
  const [paletteOpen, setPaletteOpen] = useState(false);

  const setupDone = useRef(false);
  const globalFrame = useRef<symbol | null>(null);

  if (!setupDone.current) {
    hotkeyManager.init();
    globalFrame.current = hotkeyManager.pushFrame('global', 'root');
    registry.setActiveStack(hotkeyManager.getStackSymbols());
    registerGlobalActions(navigate, globalFrame.current);
    setupDone.current = true;
  }

  useEffect(() => {
    instanceCount += 1;
    if (instanceCount > 1) {
      console.warn('[commands] CommandProvider mounted more than once — this is unsupported');
    }
    return () => {
      instanceCount -= 1;
    };
  }, []);

  useEffect(() => {
    registry.setActiveStack(hotkeyManager.getStackSymbols());
  });

  useEffect(() => {
    if (!globalFrame.current) return;
    const frame = globalFrame.current;
    const sym = hotkeyManager.bind(frame, {
      shortcut: 'mod+k',
      handler: () => {
        setPaletteOpen(o => !o);
      },
      allowInInput: true,
      id: 'meta.open-palette',
    });
    return () => hotkeyManager.unbind(frame, sym);
  }, []);

  const frame = globalFrame.current!;
  const value = useMemo(() => frame, [frame]);

  return (
    <ScopeContext.Provider value={value}>
      {children}
      <CommandPalette open={paletteOpen} onOpenChange={setPaletteOpen} />
    </ScopeContext.Provider>
  );
}
