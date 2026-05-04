import { type ReactNode, useEffect, useMemo, useState } from 'react';
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
  const [globalFrame, setGlobalFrame] = useState<symbol | null>(null);

  useEffect(() => {
    instanceCount += 1;
    if (instanceCount > 1) {
      console.warn('[commands] CommandProvider mounted more than once — this is unsupported');
    }
    hotkeyManager.init();
    const frame = hotkeyManager.pushFrame('global', 'root');
    registry.setActiveStack(hotkeyManager.getStackSymbols());
    const disposeGlobalActions = registerGlobalActions(navigate, frame);
    const paletteBinding = hotkeyManager.bind(frame, {
      shortcut: 'mod+k',
      handler: () => {
        setPaletteOpen(o => !o);
      },
      allowInInput: true,
      id: 'meta.open-palette',
    });
    setGlobalFrame(frame);

    return () => {
      disposeGlobalActions();
      hotkeyManager.unbind(frame, paletteBinding);
      hotkeyManager.popFrame(frame);
      registry.setActiveStack(hotkeyManager.getStackSymbols());
      instanceCount -= 1;
    };
  }, [navigate]);

  useEffect(() => {
    if (!globalFrame) return;
    registry.setActiveStack(hotkeyManager.getStackSymbols());
  }, [globalFrame]);

  const value = useMemo(() => globalFrame, [globalFrame]);

  if (!value) {
    return null;
  }

  return (
    <ScopeContext.Provider value={value}>
      {children}
      <CommandPalette open={paletteOpen} onOpenChange={setPaletteOpen} />
    </ScopeContext.Provider>
  );
}
