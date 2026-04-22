import { useEffect, useMemo, useState, type ReactNode } from 'react';
import { ScopeContext } from '../../lib/commands/ScopeContext';
import { hotkeyManager } from '../../lib/commands/hotkeyManager';
import type { ScopeKind } from '../../lib/commands/types';

interface Props {
  id: string;
  kind?: ScopeKind;
  children: ReactNode;
}

export default function CommandScope({ id, kind = 'page', children }: Props) {
  const [frame] = useState(() => hotkeyManager.pushFrame(kind, id));

  useEffect(() => {
    return () => {
      hotkeyManager.popFrame(frame);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const value = useMemo(() => frame, [frame]);
  return <ScopeContext.Provider value={value}>{children}</ScopeContext.Provider>;
}
