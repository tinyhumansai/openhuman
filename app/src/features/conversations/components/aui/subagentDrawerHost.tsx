import { createContext, type ReactNode, useContext, useMemo } from 'react';

export interface SubagentDrawerHostValue {
  /** Opens the host's `SubagentDrawer` on a delegation, by spawn `taskId`. */
  open: (taskId: string) => void;
  /**
   * Whether the drawer can actually show that delegation.
   *
   * The host resolves a `taskId` against the thread's live tool timeline and
   * renders nothing when it is absent, so a delegation replayed from the
   * settled core transcript would otherwise get a button that opens an empty
   * sheet. Asking the host keeps that knowledge where it already lives, and
   * keeps this seam's consumers - which are tool parts, rendered by
   * assistant-ui in contexts that do not all have a Redux store - free of a
   * store subscription of their own.
   */
  canOpen: (taskId: string) => boolean;
}

/**
 * The host that owns the sub-agent drawer's disclosure state.
 *
 * A context rather than a prop because the consumer is a *tool part*: the
 * delegation card is rendered by assistant-ui from inside the transcript, many
 * layers below anything the host passes props to, while the drawer belongs to
 * `Conversations`. `null` outside a provider, which is what every read-only
 * mount of the card (the drawer itself, past-turn insights) wants: no host, no
 * "View full processing" affordance.
 */
const SubagentDrawerHostContext = createContext<SubagentDrawerHostValue | null>(null);

export function SubagentDrawerHost({
  onOpenSubagent,
  canOpenSubagent,
  children,
}: {
  onOpenSubagent?: ((taskId: string) => void) | undefined;
  canOpenSubagent?: ((taskId: string) => boolean) | undefined;
  children: ReactNode;
}) {
  const value = useMemo<SubagentDrawerHostValue | null>(
    () =>
      onOpenSubagent ? { open: onOpenSubagent, canOpen: canOpenSubagent ?? (() => true) } : null,
    [onOpenSubagent, canOpenSubagent]
  );
  return (
    <SubagentDrawerHostContext.Provider value={value}>
      {children}
    </SubagentDrawerHostContext.Provider>
  );
}

export function useSubagentDrawerHost(): SubagentDrawerHostValue | null {
  return useContext(SubagentDrawerHostContext);
}
