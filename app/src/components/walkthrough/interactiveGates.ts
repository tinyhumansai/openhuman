import type { Step } from 'react-joyride';

import { store } from '../../store';

/**
 * An interactive gate blocks the "Next" button on a tour step until the
 * user completes a real action (or clicks "Skip this step").
 */
export interface InteractiveGate {
  /** Unique identifier, referenced by `step.data.gateId`. */
  id: string;
  /** Prompt shown in the tooltip while the gate is blocking. */
  label: string;
  /** Text for the skip button. */
  skipLabel: string;
  /** Synchronous check against current Redux state. */
  isComplete: () => boolean;
  /** How often (ms) the tooltip re-checks completion. Default 1000. */
  pollIntervalMs?: number;
}

// ── Gate definitions ──────────────────────────────────────────────────────

export const GATE_CONNECT_SKILL: InteractiveGate = {
  id: 'connect-skill',
  label: 'Connect at least one app to continue',
  skipLabel: "Skip — I'll do this later",
  isComplete: () => {
    const state = store.getState();
    return state.accounts.order.length > 0;
  },
};

export const GATE_SEND_MESSAGE: InteractiveGate = {
  id: 'send-message',
  label: 'Send your first message to continue',
  skipLabel: "Skip — I'll explore later",
  isComplete: () => {
    const state = store.getState();
    const allMessages = Object.values(state.thread.messagesByThreadId).flat();
    return allMessages.some(m => m.sender === 'user');
  },
};

/** Registry of all gates, keyed by id. */
export const GATES: Record<string, InteractiveGate> = {
  [GATE_CONNECT_SKILL.id]: GATE_CONNECT_SKILL,
  [GATE_SEND_MESSAGE.id]: GATE_SEND_MESSAGE,
};

/** Look up the interactive gate attached to a Joyride step (if any). */
export function getStepGate(step: Step): InteractiveGate | null {
  const gateId = (step.data as { gateId?: string } | undefined)?.gateId;
  return gateId ? (GATES[gateId] ?? null) : null;
}
