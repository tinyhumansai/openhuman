import { useEffect, useState } from 'react';

import type { InteractiveGate } from './interactiveGates';

const DEFAULT_POLL_MS = 1000;

/**
 * Polls `gate.isComplete()` at a regular interval and returns a reactive
 * boolean. Returns `true` immediately when there is no gate (non-gated step).
 *
 * The store is read via `gate.isComplete()` which calls `store.getState()`
 * directly — this avoids needing the Redux Provider context, which Joyride
 * tooltip components don't have access to.
 */
export function useGatePoller(gate: InteractiveGate | null): boolean {
  const [complete, setComplete] = useState<boolean>(() => gate?.isComplete() ?? true);

  useEffect(() => {
    if (!gate) {
      setComplete(true);
      return;
    }

    // Immediately check — the gate might already be satisfied.
    if (gate.isComplete()) {
      setComplete(true);
      return;
    }

    setComplete(false);

    const interval = setInterval(() => {
      if (gate.isComplete()) {
        setComplete(true);
        clearInterval(interval);
      }
    }, gate.pollIntervalMs ?? DEFAULT_POLL_MS);

    return () => clearInterval(interval);
  }, [gate]);

  return complete;
}
