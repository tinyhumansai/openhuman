import { createContext, type ReactNode, useContext, useMemo } from 'react';

import type { TurnProcessTrail } from '../../../../providers/assistantUiMessages';

export interface TurnFooterHostValue {
  /** Opens the host's process rail on one settled turn's trail. */
  open: (trail: TurnProcessTrail) => void;
}

/**
 * The host that owns the process rail the turn footer opens.
 *
 * A context rather than a prop for the same reason
 * {@link import('./subagentDrawerHost').SubagentDrawerHost} is one: the
 * consumer is rendered by assistant-ui from *inside* the transcript, many
 * layers below anything the host passes props to, while the panel belongs to
 * `Conversations`.
 *
 * The trail travels with the click rather than being looked up by request id.
 * `useOpenHumanExternalStore` has already paged the core transcript projection
 * and split it per turn; the footer is handed those same array references on
 * the message's own metadata, so opening the rail costs no second fetch and no
 * second copy of the paging policy.
 *
 * `null` outside a provider, which is what every read-only mount wants: no
 * host, no footer.
 */
const TurnFooterHostContext = createContext<TurnFooterHostValue | null>(null);

export function TurnFooterHost({
  onOpenTurnProcess,
  children,
}: {
  onOpenTurnProcess?: ((trail: TurnProcessTrail) => void) | undefined;
  children: ReactNode;
}) {
  const value = useMemo<TurnFooterHostValue | null>(
    () => (onOpenTurnProcess ? { open: onOpenTurnProcess } : null),
    [onOpenTurnProcess]
  );
  return <TurnFooterHostContext.Provider value={value}>{children}</TurnFooterHostContext.Provider>;
}

export function useTurnFooterHost(): TurnFooterHostValue | null {
  return useContext(TurnFooterHostContext);
}
