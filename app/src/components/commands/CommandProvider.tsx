import { type ReactNode, useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { type GlobalActionHandlers, registerGlobalActions } from '../../lib/commands/globalActions';
import { hotkeyManager } from '../../lib/commands/hotkeyManager';
import { registry } from '../../lib/commands/registry';
import { ScopeContext } from '../../lib/commands/ScopeContext';
import { useAppDispatch } from '../../store/hooks';
import { toggleSidebar } from '../../store/layoutSlice';
import { APP_SHELL_LAYOUT_ID } from '../layout/shell/RootShellLayout';
import { useNewChat } from '../layout/shell/useNewChat';
import KeyboardShortcutsModal from '../shortcuts/KeyboardShortcutsModal';
import CommandPalette from './CommandPalette';

let instanceCount = 0;

interface Props {
  children: ReactNode;
}

export default function CommandProvider({ children }: Props) {
  const navigate = useNavigate();
  const dispatch = useAppDispatch();
  const newChat = useNewChat();
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [globalFrame, setGlobalFrame] = useState<symbol | null>(null);

  // Latest handlers held in a ref so the registration effect runs exactly once
  // (on mount) — `useHomeNav`'s callback changes as the route/threads change,
  // and re-running the effect would tear down and rebuild the whole hotkey
  // frame on every navigation.
  const handlersRef = useRef<GlobalActionHandlers>(null as unknown as GlobalActionHandlers);
  handlersRef.current = {
    navigate: path => navigate(path),
    newChat,
    toggleSidebar: () => dispatch(toggleSidebar({ id: APP_SHELL_LAYOUT_ID })),
    // The two overlays are mutually exclusive: opening one dismisses the other.
    openPalette: () => {
      setShortcutsOpen(false);
      setPaletteOpen(o => !o);
    },
    openShortcuts: () => {
      setPaletteOpen(false);
      setShortcutsOpen(o => !o);
    },
  };

  useEffect(() => {
    instanceCount += 1;
    if (instanceCount > 1) {
      console.warn('[commands] CommandProvider mounted more than once — this is unsupported');
    }
    hotkeyManager.init();
    const frame = hotkeyManager.pushFrame('global', 'root');
    registry.setActiveStack(hotkeyManager.getStackSymbols());

    // Stable indirection: registered handlers always call through to the latest
    // ref value, so closures captured at registration time stay current.
    const stableHandlers: GlobalActionHandlers = {
      navigate: path => handlersRef.current.navigate(path),
      newChat: () => handlersRef.current.newChat(),
      toggleSidebar: () => handlersRef.current.toggleSidebar(),
      openPalette: () => handlersRef.current.openPalette(),
      openShortcuts: () => handlersRef.current.openShortcuts(),
    };
    const disposeGlobalActions = registerGlobalActions(stableHandlers, frame);
    setGlobalFrame(frame);

    return () => {
      disposeGlobalActions();
      hotkeyManager.popFrame(frame);
      registry.setActiveStack(hotkeyManager.getStackSymbols());
      instanceCount -= 1;
    };
    // Mount-once: handlers are reached via `handlersRef`, not deps.
  }, []);

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
      <KeyboardShortcutsModal open={shortcutsOpen} onOpenChange={setShortcutsOpen} />
    </ScopeContext.Provider>
  );
}
