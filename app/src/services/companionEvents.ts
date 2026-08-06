import { listen, type UnlistenFn } from '@tauri-apps/api/event';

import {
  type CompanionState,
  type CompanionStateChangedEvent,
  setCompanionState,
} from '../store/companionSlice';
import { store } from '../store/index';
import { isTauri } from '../utils/tauriCommands/common';

/**
 * Companion state changes are now delivered by the Tauri shell as a
 * `companion://state_changed` event (camelCase payload
 * `{ sessionId, state, previousState }`), replacing the old core Socket.IO
 * `companion:state_changed` bridge. This module parses that payload and pumps
 * it into the companion Redux slice.
 */

export const COMPANION_STATE_CHANGED_EVENT = 'companion://state_changed';

const COMPANION_STATES: ReadonlySet<string> = new Set([
  'idle',
  'listening',
  'thinking',
  'speaking',
  'error',
]);

/**
 * Validate + normalize a raw `companion://state_changed` payload into a typed
 * event, or `null` if the shape is invalid. Tolerant of a missing/invalid
 * `previousState` (defaults to `'idle'`).
 */
export function parseCompanionStateChangedEvent(value: unknown): CompanionStateChangedEvent | null {
  if (!value || typeof value !== 'object') return null;
  const obj = value as Record<string, unknown>;
  if (typeof obj.sessionId !== 'string') return null;
  if (typeof obj.state !== 'string' || !COMPANION_STATES.has(obj.state)) return null;

  const previousState =
    typeof obj.previousState === 'string' && COMPANION_STATES.has(obj.previousState)
      ? (obj.previousState as CompanionState)
      : 'idle';

  return { sessionId: obj.sessionId, state: obj.state as CompanionState, previousState };
}

/**
 * Subscribe to the shell's companion state-changed events and dispatch them into
 * the companion slice. No-op (resolves to a noop unlisten) outside Tauri.
 */
export async function subscribeCompanionStateChanged(): Promise<UnlistenFn> {
  if (!isTauri()) {
    return () => {};
  }
  return listen(COMPANION_STATE_CHANGED_EVENT, event => {
    const parsed = parseCompanionStateChangedEvent(event.payload);
    if (!parsed) {
      console.warn('[companion] state_changed dropped — invalid payload shape');
      return;
    }
    store.dispatch(setCompanionState(parsed));
  });
}
