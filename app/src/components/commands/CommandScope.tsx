import { type ReactNode, useEffect, useMemo, useState } from 'react';

import { hotkeyManager } from '../../lib/commands/hotkeyManager';
import { ScopeContext } from '../../lib/commands/ScopeContext';
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
  }, [frame]);

  const value = useMemo(() => frame, [frame]);
  return <ScopeContext.Provider value={value}>{children}</ScopeContext.Provider>;
}
